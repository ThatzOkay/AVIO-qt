pub const CARLINKIT_VID: u16 = 0x1314;
pub const CARLINKIT_PIDS: [u16; 2] = [0x1520, 0x1521];

pub fn is_carlinkit_dongle(vid: Option<u16>, pid: Option<u16>) -> bool {
    match (vid, pid) {
        (Some(vid), Some(pid)) => vid == CARLINKIT_VID && CARLINKIT_PIDS.contains(&pid),
        _ => false,
    }
}

// Generic Android USB DAB+ stick (WRadio C100). Uses V-USB's shared obdev VID/PID, so this
// is only a hint that the stick is present, not proof - other V-USB devices share it.
pub const WRADIO_C100_VID: u16 = 0x16c0;
pub const WRADIO_C100_PID: u16 = 0x05dc;

pub fn is_wradio_c100(vid: u16, pid: u16) -> bool {
    vid == WRADIO_C100_VID && pid == WRADIO_C100_PID
}
