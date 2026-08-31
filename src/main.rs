//! The `kairo` binary: load the configuration, install logging, then serve until a signal arrives.

use std::future::Future;
use std::pin::pin;
use std::process::ExitCode;
use std::sync::LazyLock;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::Router;
use hyper::body::Incoming;
use hyper::Request;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tower_service::Service;

use kairo::node::AppState;
use kairo::CONFIG;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    disable_thp();

    // The only way this fails is a provider already being installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();

    LazyLock::force(&CONFIG);

    // The guard flushes what the file sink buffered when dropped, so it is held for all of `main`.
    let _logging = kairo::utils::init(&CONFIG.logging);

    let bind = format!("{}:{}", CONFIG.server.address, CONFIG.server.port);
    let http2 = CONFIG.server.http2.enabled;

    let state = AppState::new(CONFIG.clone());

    let sources = state.source_names();
    if sources.is_empty() {
        tracing::warn!("No audio sources are enabled; track loading will return no matches");
    } else {
        tracing::info!("Registered source managers: {}", sources.join(", "));
    }

    let lyrics_providers = state.lyrics_provider_names();
    if !lyrics_providers.is_empty() {
        tracing::info!(
            "Registered lyrics providers: {}",
            lyrics_providers.join(", ")
        );
    }

    let app = kairo::rest::app(state);

    let listener = match TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("failed to bind {bind}: {err}");
            return ExitCode::FAILURE;
        }
    };

    if http2 {
        tracing::info!("Kairo listening on http://{bind} (HTTP/1.1 and h2c)");
    } else {
        tracing::info!("Kairo listening on http://{bind}");
    }

    serve(listener, app, http2, shutdown_signal()).await;
    ExitCode::SUCCESS
}

// Opt the process out of transparent huge pages.
//
// The allocator's arenas are 2 MiB aligned, so a host set to `always` has khugepaged collapse them
// into huge pages. Freed slices are purged below that granularity, the collapse faults every hole
// back in, and the resident set climbs towards a 2 MiB granular peak it never gives back.
//
// `KAIRO_ALLOW_THP` leaves the host setting alone, for a workload that would rather have the TLB.
#[cfg(target_os = "linux")]
fn disable_thp() {
    if std::env::var_os("KAIRO_ALLOW_THP").is_some() {
        return;
    }
    // Only fails on kernels below 3.15, which have nothing to opt out of.
    unsafe { libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0) };
}

#[cfg(not(target_os = "linux"))]
fn disable_thp() {}

// Accept connections until `shutdown` resolves, then let the open ones finish.
//
// Hand-rolled rather than `axum::serve` because each connection needs its peer address attached for
// the request log, HTTP upgrades have to survive for WebSockets, and h2c is only offered when
// configured.
async fn serve(
    listener: TcpListener,
    app: Router,
    http2: bool,
    shutdown: impl Future<Output = ()>,
) {
    let mut sniffing = auto::Builder::new(TokioExecutor::new());
    sniffing.http2().enable_connect_protocol();
    let http1 = hyper::server::conn::http1::Builder::new();

    let (stop, _) = watch::channel(false);
    let mut conns: JoinSet<()> = JoinSet::new();
    let mut shutdown = pin!(shutdown);

    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(conn) => conn,
                // Usually the file descriptor limit. Pause briefly rather than spin on it.
                Err(err) => {
                    tracing::debug!("failed to accept a connection: {err}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };

        let app = app.clone();
        let service = hyper::service::service_fn(move |mut req: Request<Incoming>| {
            req.extensions_mut().insert(ConnectInfo(peer));
            app.clone().call(req)
        });
        let io = TokioIo::new(stream);
        let mut stopping = stop.subscribe();

        if http2 {
            let conn = sniffing
                .serve_connection_with_upgrades(io, service)
                .into_owned();
            conns.spawn(async move {
                let mut conn = pin!(conn);
                let result = tokio::select! {
                    result = conn.as_mut() => result,
                    _ = stopping.changed() => {
                        conn.as_mut().graceful_shutdown();
                        conn.as_mut().await
                    }
                };
                if let Err(err) = result {
                    tracing::debug!("connection from {peer} failed: {err}");
                }
            });
        } else {
            let conn = http1.serve_connection(io, service).with_upgrades();
            conns.spawn(async move {
                let mut conn = pin!(conn);
                let result = tokio::select! {
                    result = conn.as_mut() => result,
                    _ = stopping.changed() => {
                        conn.as_mut().graceful_shutdown();
                        conn.as_mut().await
                    }
                };
                if let Err(err) = result {
                    tracing::debug!("connection from {peer} failed: {err}");
                }
            });
        }
    }

    // Stop taking new connections, ask the open ones to wind down, then wait them out.
    drop(listener);
    let _ = stop.send(true);
    while conns.join_next().await.is_some() {}
}
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let sigterm = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c  => tracing::info!("received Ctrl-C, shutting down"),
        () = sigterm => tracing::info!("received SIGTERM, shutting down"),
    }
}
