pub struct AudioFormat {
    pub frequency: u32,
    pub channels: u32,
    pub bit_depth: u32,
    pub format: Option<String>,
    pub mime_type: Option<String>,
}

pub fn decode_type_map(key: i32) -> Option<AudioFormat> {
    match key {
        1 | 2 => Some(AudioFormat {
            frequency: 44100,
            channels: 2,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=44100; channels=2".to_string()),
        }),
        3 => Some(AudioFormat {
            frequency: 8000,
            channels: 1,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=8000; channels=1".to_string()),
        }),
        4 => Some(AudioFormat {
            frequency: 48000,
            channels: 2,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=48000; channels=2".to_string()),
        }),
        5 => Some(AudioFormat {
            frequency: 16000,
            channels: 1,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=16000; channels=1".to_string()),
        }),
        6 => Some(AudioFormat {
            frequency: 24000,
            channels: 1,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=24000; channels=1".to_string()),
        }),
        7 => Some(AudioFormat {
            frequency: 16000,
            channels: 2,
            bit_depth: 16,
            format: Some("s16le".to_string()),
            mime_type: Some("audio/L16; rate=16000; channels=2".to_string()),
        }),
        _ => None,
    }
}
