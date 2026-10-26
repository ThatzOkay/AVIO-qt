//! FIG type 1 extension parsers (labels for display). Each extension
//! corresponds to one ETS 300 401 clause, cited in its own doc comment.

use crate::fic::fig::{parse_label, DabLabel, Fig01Header, FigCursor};
use crate::registered_tables::CharacterSet;

// ---------------------------------------------------------------------
// FIG 1/0 -- ETS 300 401 clause 8.1.13: Ensemble label
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0100 {
    pub ensemble_id: u16,
    pub label: DabLabel,
    pub charset: CharacterSet,
}

pub fn parse_fig_01_00(fig_data: &[u8], header: Fig01Header) -> Vec<Fig0100> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 18 {
        let Some(ensemble_id) = cur.u16() else { break };
        let Some(field) = cur.take(18) else { break };
        let Some(label) = parse_label(field, header.charset) else {
            break;
        };
        out.push(Fig0100 {
            ensemble_id,
            label,
            charset: header.charset,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 1/1 -- ETS 300 401 clause 8.1.14.1: Programme service label
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0101 {
    pub programme_service_id: u16,
    pub label: DabLabel,
    pub charset: CharacterSet,
}

pub fn parse_fig_01_01(fig_data: &[u8], header: Fig01Header) -> Vec<Fig0101> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 18 {
        let Some(programme_service_id) = cur.u16() else {
            break;
        };
        let Some(field) = cur.take(18) else { break };
        let Some(label) = parse_label(field, header.charset) else {
            break;
        };
        out.push(Fig0101 {
            programme_service_id,
            label,
            charset: header.charset,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 1/4 -- ETS 300 401 clause 8.1.14.3: Service component label
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0104 {
    pub is_programme_service: bool,
    pub service_id: u32,
    pub sc_id_s: u8,
    pub label: DabLabel,
    pub charset: CharacterSet,
}

pub fn parse_fig_01_04(fig_data: &[u8], header: Fig01Header) -> Vec<Fig0104> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 19 {
        let Some(b) = cur.u8() else { break };
        let is_programme_service = (b & 0x80) == 0;
        let sc_id_s = b & 0x0F;

        let service_id = if is_programme_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(field) = cur.take(18) else { break };
        let Some(label) = parse_label(field, header.charset) else {
            break;
        };

        out.push(Fig0104 {
            is_programme_service,
            service_id,
            sc_id_s,
            label,
            charset: header.charset,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 1/5 -- ETS 300 401 clause 8.1.14.2: Data service label
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0105 {
    pub data_service_id: u32,
    pub label: DabLabel,
    pub charset: CharacterSet,
}

pub fn parse_fig_01_05(fig_data: &[u8], header: Fig01Header) -> Vec<Fig0105> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 22 {
        let Some(data_service_id) = cur.u32() else {
            break;
        };
        let Some(field) = cur.take(18) else { break };
        let Some(label) = parse_label(field, header.charset) else {
            break;
        };
        out.push(Fig0105 {
            data_service_id,
            label,
            charset: header.charset,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 1/6 -- ETS 300 401 clause 8.1.14.4: X-PAD user application label
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0106 {
    pub is_programme_service: bool,
    pub service_id: u32,
    pub sc_id_s: u8,
    pub xpad_app_type: u8,
    pub label: DabLabel,
    pub charset: CharacterSet,
}

/// Parses FIG 1/6.
///
/// The reference implementation's constructor for this extension never
/// actually calls its own `parseFigData` (every other `Fig_0x_Ext_xx`
/// constructor does) -- so in the original, X-PAD user application labels
/// are parsed nowhere and every field is left default/uninitialized. Wired
/// up properly here.
pub fn parse_fig_01_06(fig_data: &[u8], header: Fig01Header) -> Vec<Fig0106> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 19 {
        let Some(b) = cur.u8() else { break };
        let is_programme_service = (b & 0x80) == 0;
        let sc_id_s = b & 0x0F;

        let service_id = if is_programme_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let xpad_app_type = b & 0x1F;

        let Some(field) = cur.take(18) else { break };
        let Some(label) = parse_label(field, header.charset) else {
            break;
        };

        out.push(Fig0106 {
            is_programme_service,
            service_id,
            sc_id_s,
            xpad_app_type,
            label,
            charset: header.charset,
        });
    }
    out
}
