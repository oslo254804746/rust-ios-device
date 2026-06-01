use bytes::{BufMut, Bytes, BytesMut};

use super::ValeriaError;

const ANNEX_B_START_CODE: &[u8; 4] = b"\x00\x00\x00\x01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Frame {
    pub nalu_data: Bytes,
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub width: u32,
    pub height: u32,
    pub pts_value: i64,
    pub pts_scale: i32,
}

impl H264Frame {
    pub fn from_avcc(nalu_data: Bytes) -> Self {
        Self {
            nalu_data,
            sps: None,
            pps: None,
            width: 0,
            height: 0,
            pts_value: 0,
            pts_scale: 0,
        }
    }

    pub fn is_keyframe(&self) -> bool {
        self.sps.is_some() && self.pps.is_some()
    }

    pub fn pts_ns(&self) -> Option<i64> {
        if self.pts_scale == 0 {
            return None;
        }
        Some(self.pts_value.saturating_mul(1_000_000_000) / i64::from(self.pts_scale))
    }

    pub fn to_annex_b(&self) -> Result<Bytes, ValeriaError> {
        let mut out = BytesMut::new();
        if let Some(sps) = &self.sps {
            out.put_slice(ANNEX_B_START_CODE);
            out.put_slice(sps);
        }
        if let Some(pps) = &self.pps {
            out.put_slice(ANNEX_B_START_CODE);
            out.put_slice(pps);
        }

        let data = self.nalu_data.as_ref();
        let mut pos = 0usize;
        while pos < data.len() {
            if pos + 4 > data.len() {
                return Err(ValeriaError::Protocol(format!(
                    "AVCC NALU length header truncated at byte {pos}"
                )));
            }
            let nalu_len =
                u32::from_be_bytes(data[pos..pos + 4].try_into().expect("slice length checked"))
                    as usize;
            pos += 4;
            if nalu_len == 0 || pos + nalu_len > data.len() {
                return Err(ValeriaError::Protocol(format!(
                    "AVCC NALU length {nalu_len} exceeds remaining {} bytes",
                    data.len().saturating_sub(pos)
                )));
            }
            out.put_slice(ANNEX_B_START_CODE);
            out.put_slice(&data[pos..pos + nalu_len]);
            pos += nalu_len;
        }

        Ok(out.freeze())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::H264Frame;

    #[test]
    fn converts_avcc_nalus_to_annex_b_with_parameter_sets() {
        let frame = H264Frame {
            nalu_data: Bytes::from_static(&[
                0x00, 0x00, 0x00, 0x03, 0x65, 0x88, 0x84, 0x00, 0x00, 0x00, 0x02, 0x41, 0x9a,
            ]),
            sps: Some(Bytes::from_static(&[0x67, 0x42, 0x00, 0x1f])),
            pps: Some(Bytes::from_static(&[0x68, 0xce, 0x3c, 0x80])),
            width: 1179,
            height: 2556,
            pts_value: 123,
            pts_scale: 1000,
        };

        let annex_b = frame.to_annex_b().unwrap();
        assert_eq!(
            annex_b.as_ref(),
            &[
                0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80, 0, 0, 0, 1,
                0x65, 0x88, 0x84, 0, 0, 0, 1, 0x41, 0x9a,
            ]
        );
        assert!(frame.is_keyframe());
        assert_eq!(frame.pts_ns(), Some(123_000_000));
    }

    #[test]
    fn rejects_truncated_avcc_nalu() {
        let frame = H264Frame::from_avcc(Bytes::from_static(&[0x00, 0x00, 0x00, 0x05, 0x65, 0x88]));

        let err = frame.to_annex_b().expect_err("truncated NALU must fail");
        assert!(err.to_string().contains("AVCC NALU length"));
    }
}
