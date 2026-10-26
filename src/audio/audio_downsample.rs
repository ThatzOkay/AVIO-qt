pub struct DownsampleOptions {
    pub in_sample_rate: i32,
    pub in_channels: i32,
    pub out_sample_rate: Option<i32>,
}

pub fn downsample_to_mono(pcm: Vec<i16>, options: DownsampleOptions) -> Vec<i16> {
    let in_sample_rate = options.in_sample_rate;
    let in_channels = options.in_channels as usize;
    let out_sample_rate = options.out_sample_rate.unwrap_or(in_sample_rate);

    if pcm.is_empty() || in_channels == 0 {
        return vec![];
    }

    if in_channels == 1 && in_sample_rate == out_sample_rate {
        return pcm;
    }

    let frames_in = pcm.len() / in_channels;
    if frames_in == 0 {
        return vec![];
    }

    let ratio = in_sample_rate as f64 / out_sample_rate as f64;
    let frames_out = (frames_in as f64 / ratio) as usize;
    if frames_out == 0 {
        return vec![];
    }

    let mut out = Vec::with_capacity(frames_out);

    for i in 0..frames_out {
        let src_frame = (i as f64 * ratio) as usize;
        let base_index = src_frame * in_channels;
        if base_index + in_channels > pcm.len() {
            break;
        }

        let sum: i32 = (0..in_channels).map(|c| pcm[base_index + c] as i32).sum();

        out.push((sum / in_channels as i32) as i16);
    }

    out
}
