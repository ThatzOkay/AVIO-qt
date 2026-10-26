//! MOT slideshow decode -> `SlideshowUpdate` events -- the DAB "album art"
//! equivalent (station logo / now-playing cover art). Ported from
//! `slideshowdecoder.cpp/h`.

use crate::mot::MotData;

/// `SlideshowDecoder::IMAGE_MIMETYPE`, indexed by `MOT_CONTENT_SUBTYPE_IMAGE`.
pub const IMAGE_MIMETYPE: [&str; 4] = ["image/gif", "image/jpeg", "image/bmp", "image/png"];

/// One decoded slideshow image. Mirrors `DabSlideshow`.
///
/// `slideshow_data` derives the default `serde` `Vec<u8>` representation
/// (a JSON array of numbers) rather than base64 -- functionally correct
/// for IPC (the plugin layer, Phase 8, forwards this as-is via
/// `app_handle.emit`), but meaningfully larger on the wire than base64
/// would be for a several-KB image; worth revisiting with a
/// `#[serde(with = "...")]` base64 helper if that overhead ever matters in
/// practice, not done here since `RUST_PORT_PROMPT.md` doesn't call for a
/// new dependency to do it.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowUpdate {
    pub is_categorized: bool,
    pub content_name: String,
    pub category_id: u8,
    pub slide_id: u8,
    pub category_title: String,
    pub click_through_url: String,
    pub alternative_location_url: String,
    pub mime_type: String,
    pub content_sub_type: u16,
    pub slideshow_data: Vec<u8>,
}

/// `SlideshowDecoder::motApplicationDataInput`. `mot_data` must have
/// `content_type == MotContentType::Image` (mirrors the reference, where
/// `MotDecoder` only ever calls into this decoder for `IMAGE` content --
/// see `mot::MotDecoder::mot_data_input`'s own doc comment).
pub fn decode(mot_data: &MotData) -> SlideshowUpdate {
    let mut slide = SlideshowUpdate::default();

    for (&param_id, bytes) in &mot_data.header_params {
        match param_id {
            0x0C => {
                // ContentName: byte 0 is a charset indicator, parsed by
                // the reference but -- like FIG/DLS labels before their
                // own charset wiring was fixed in this port -- never
                // actually applied there (raw bytes go straight into the
                // string). Left as raw bytes here too rather than
                // guessing which of this crate's other charset-decode
                // paths would apply, since MOT ContentName's charset
                // field uses a different registered table than FIG/DLS's
                // (ETSI TS 101 756 table 8) that nothing else in this
                // crate has ported yet.
                if let Some((_charset_indicator, name)) = bytes.split_first() {
                    slide.content_name = String::from_utf8_lossy(name).into_owned();
                }
            }
            0x25 => {
                // CategoryId/SlideId
                if let [category_id, slide_id, ..] = bytes[..] {
                    slide.category_id = category_id;
                    slide.slide_id = slide_id;
                    slide.is_categorized = true;
                }
            }
            0x26 => slide.category_title = String::from_utf8_lossy(bytes).into_owned(),
            0x27 => slide.click_through_url = String::from_utf8_lossy(bytes).into_owned(),
            0x28 => slide.alternative_location_url = String::from_utf8_lossy(bytes).into_owned(),
            // 0x04 ExpireTime / 0x05 TriggerTime: the reference parses
            // only as far as a validity flag it logs and never actually
            // stores on the slide (`//TODO expiretime/triggertime
            // parameters parsing and set slide param`, its own comment) --
            // nothing to port here that isn't already a stated TODO
            // upstream. 0x29 Alert: the reference's handler is empty.
            _ => {}
        }
    }

    slide.slideshow_data = mot_data.body_data.clone();
    slide.content_sub_type = mot_data.content_subtype;
    // Unlike the reference's direct `IMAGE_MIMETYPE[contentSubtype]` index
    // (undefined behaviour in C++ if contentSubtype > 3, since that array
    // is fixed at 4 entries with no bounds check), an out-of-range
    // subtype here just yields an empty MIME type -- matches this crate's
    // general "bail out cleanly rather than index out of bounds" handling
    // of malformed real-world broadcast data.
    slide.mime_type = IMAGE_MIMETYPE
        .get(mot_data.content_subtype as usize)
        .copied()
        .unwrap_or("")
        .to_string();

    slide
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mot::MotDecoder;

    /// One MOT_HEADER + one MOT_BODY_UNSCRAMBLED datagroup for a 5-byte
    /// PNG image, transport id 0x1234, with a ContentName ("pic.png")
    /// header extension parameter -- exercises `mot::MotDecoder` and
    /// `slideshow::decode` together end to end.
    #[test]
    fn decodes_mot_header_and_body_into_slideshow_update() {
        let header: Vec<u8> = vec![
            0x13, // extFlag=0,crcFlag=0,segFlag=0,userAccessFlag=1,type=MOT_HEADER(3)
            0x00, // continuityIdx/repetitionIdx (unused)
            0x12, // userAccess: transportIdFlag=1, lengthIndicator=2
            0x12, 0x34, // transport id 0x1234
            0x00, 0x00, // segmentation header: repCnt=0, segmentSize=0 (unused for HEADER)
            // Header core (7 bytes): bodySize=5, contentType=IMAGE(2), contentSubtype=PNG(3)
            0x00, 0x00, 0x00, 0x50, 0x00, 0x04, 0x03,
            // Header extension: ContentName (id 0x0C), short-form length=8
            // (charset byte + "pic.png").
            0xCC, 0x08, 0x00, b'p', b'i', b'c', b'.', b'p', b'n', b'g',
        ];

        let body: Vec<u8> = vec![
            0x34, // extFlag=0,crcFlag=0,segFlag=1,userAccessFlag=1,type=MOT_BODY_UNSCRAMBLED(4)
            0x00, // continuityIdx/repetitionIdx (unused)
            0x80, 0x00, // segNum=0, isLast=1
            0x12, // userAccess: transportIdFlag=1, lengthIndicator=2
            0x12, 0x34, // transport id 0x1234 -- must match the header's
            0x00, 0x05, // segmentation header: repCnt=0, segmentSize=5
            1, 2, 3, 4, 5, // body data
        ];

        let mut decoder = MotDecoder::new();
        assert!(decoder.mot_data_input(&header).is_none());
        let image = decoder
            .mot_data_input(&body)
            .expect("completed IMAGE MOT object expected");

        let slide = decode(&image);
        assert_eq!(slide.content_name, "pic.png");
        assert_eq!(slide.mime_type, "image/png");
        assert_eq!(slide.content_sub_type, 3);
        assert_eq!(slide.slideshow_data, vec![1, 2, 3, 4, 5]);
        assert!(!slide.is_categorized);
    }
}
