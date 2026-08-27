pub mod protocol;
pub mod sponsorblock;

use mimalloc::MiMalloc;

#[global_allocator]
static ALLOCATOR: MiMalloc = MiMalloc;
