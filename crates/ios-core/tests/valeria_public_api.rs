#![cfg(feature = "valeria")]

use ios_core::valeria::{CaptureOptions, H264Frame, ValeriaError};

#[test]
fn valeria_public_api_types_are_available() {
    let options = CaptureOptions::default();
    assert_eq!(options.queue_capacity, 90);

    let frame = H264Frame::from_avcc(bytes::Bytes::new());
    assert_eq!(frame.width, 0);
    assert_eq!(frame.height, 0);

    let err = ValeriaError::Protocol("bad packet".to_string());
    assert!(err.to_string().contains("protocol error: bad packet"));
}
