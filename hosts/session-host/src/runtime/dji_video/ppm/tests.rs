use super::{PpmDecoder, PpmError};

#[test]
fn every_split_preserves_binary_pixels_and_concatenated_frames() {
    let bytes = b"P6\n2 1\n255\n \nP6\0\xffP6\n1 1\n255\n\xff\0\n";
    for split in 0..=bytes.len() {
        let mut decoder = PpmDecoder::default();
        let mut frames = decoder.push(&bytes[..split]).unwrap();
        frames.extend(decoder.push(&bytes[split..]).unwrap());
        decoder.finish().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].width, frames[0].height), (2, 1));
        assert_eq!(frames[0].rgb, b" \nP6\0\xff");
        assert_eq!(frames[1].rgb, b"\xff\0\n");
    }
}

#[test]
fn invalid_depth_dimensions_and_oversized_headers_fail_before_pixels_are_allocated() {
    for bytes in [
        b"P5\n1 1\n255\n".as_slice(),
        b"P6\n1 1\n65535\n",
        b"P6\n0 1\n255\n",
        b"P6\n4097 1\n255\n",
        b"P6\n1 2161\n255\n",
        b"P6\n4294967295 4294967295\n255\n",
    ] {
        let mut decoder = PpmDecoder::default();
        assert!(decoder.push(bytes).is_err());
        assert!(decoder.frame.is_none());
    }
    let mut decoder = PpmDecoder::default();
    assert!(matches!(
        decoder.push(&vec![b'P'; 257]),
        Err(PpmError::HeaderLimit)
    ));
}

#[test]
fn eof_distinguishes_a_complete_frame_from_a_partial_header_or_pixels() {
    for bytes in [b"P6\n".as_slice(), b"P6\n1 1\n255\n\0\0"] {
        let mut decoder = PpmDecoder::default();
        decoder.push(bytes).unwrap();
        assert!(matches!(decoder.finish(), Err(PpmError::Truncated)));
    }
    let mut decoder = PpmDecoder::default();
    let mut frames = Vec::new();
    for byte in b"P6\n1 1\n255\n\0\0\0" {
        frames.extend(decoder.push(&[*byte]).unwrap());
    }
    assert_eq!(frames.len(), 1);
    decoder.finish().unwrap();
}
