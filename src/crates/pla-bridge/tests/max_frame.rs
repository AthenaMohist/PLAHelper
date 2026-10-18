//! G4: MAX_FRAME_BYTES must be an explicit ≥4MB WS ceiling (ARCH §8.3 /
//! est §1.3) — real LIST_ALL responses measured at 368KB, well above the
//! historical 64KB-accident risk; the constant is the single source of
//! truth wired into connect_async_with_config.

use pla_bridge::connection::MAX_FRAME_BYTES;

#[test]
fn max_frame_bytes_is_at_least_4mb() {
    assert!(MAX_FRAME_BYTES >= 4 * 1024 * 1024);
}

#[test]
fn max_frame_bytes_comfortably_exceeds_measured_real_response() {
    // Largest real capture: LIST_ALL frame-listall-resp-0 = 368040 bytes.
    assert!(MAX_FRAME_BYTES > 368_040 * 4);
}
