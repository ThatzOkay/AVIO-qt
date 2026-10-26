//! FIC (Fast Information Channel) parsing: splits raw FIC bytes into FIBs,
//! CRC-checks them, and walks each FIB's FIGs into typed events.
//!
//! This is a synchronous, stateless parser by design -- unlike the
//! reference implementation, which ran this on its own background thread
//! reading off a `ConcurrentQueue`, the async architecture used here
//! drives this from whichever task already has the raw FIC bytes (see the
//! "Async/runtime" section of the port task): call [`parse_fic`] directly
//! and forward its output over a channel/broadcast if a consumer needs it
//! elsewhere. Ensemble/service database bookkeeping (which FIGs have
//! stabilized, when the ensemble is considered fully collected, ...) lives
//! in `crate::ensemble`, not here.

pub mod fig;
pub mod fig_00;
pub mod fig_01;

use fig::{Fig00Header, Fig01Header, FigCursor};

/// One parsed FIG entry, tagged by its FIG/extension number.
#[derive(Debug, Clone)]
pub enum FigEvent {
    Fig0000(fig_00::Fig0000),
    Fig0001(fig_00::BasicSubchannelOrganization),
    Fig0002(fig_00::ServiceDescription),
    Fig0003(fig_00::PacketModeServiceComponentDescription),
    Fig0004(fig_00::Fig0004),
    Fig0005(fig_00::Fig0005),
    Fig0006(fig_00::ServiceLinkingInformation),
    Fig0007(fig_00::Fig0007),
    Fig0008(fig_00::ServiceComponentGlobalDefinition),
    Fig0009(fig_00::Fig0009),
    Fig0010(fig_00::Fig0010),
    Fig0013(fig_00::UserApplicationInformation),
    Fig0014(fig_00::FecSchemeDescription),
    Fig0017(fig_00::ProgrammeTypeInformation),
    Fig0018(fig_00::AnnouncementSupport),
    Fig0019(fig_00::AnnouncementSwitching),
    Fig0020(fig_00::Fig0020),
    Fig0021(fig_00::FrequencyInformation),
    Fig0024(fig_00::OtherEnsembleServiceInformation),
    Fig0025(fig_00::OeAnnouncementSupport),
    Fig0026(fig_00::OeAnnouncementSwitching),
    Fig0100(fig_01::Fig0100),
    Fig0101(fig_01::Fig0101),
    Fig0104(fig_01::Fig0104),
    Fig0105(fig_01::Fig0105),
    Fig0106(fig_01::Fig0106),
}

/// Splits raw FIC bytes (as delivered by one FIC memory read) into 32-byte
/// FIBs, CRC-checks each, and parses every FIG inside the ones that pass
/// into a flat list of [`FigEvent`]s. Corrupted FIBs are silently dropped,
/// same as the reference (which only logs and continues).
pub fn parse_fic(fic_data: &[u8]) -> Vec<FigEvent> {
    let mut events = Vec::new();

    for fib in fic_data.chunks_exact(32) {
        if !crate::tables::crc_ccitt_check(fib) {
            continue;
        }

        parse_fib(fib, &mut events);
    }

    events
}

fn parse_fib(fib: &[u8], events: &mut Vec<FigEvent>) {
    // The last 2 bytes of a FIB are its CRC, already verified by the
    // caller -- only the preceding 30 bytes carry FIGs.
    let mut cur = FigCursor::new(&fib[..fib.len() - 2]);

    while cur.has_remaining() {
        let Some(header_byte) = cur.u8() else { break };
        let fig_type = (header_byte & 0xE0) >> 5;
        let fig_length = header_byte & 0x1F;

        // FIG type 7, a length of 31, or a length of 0 all mark padding /
        // end-of-FIB, matching the reference's loop-terminating condition.
        if fig_type == 7 || fig_length == 31 || fig_length == 0 {
            break;
        }

        let Some(fig_data) = cur.take(fig_length as usize) else {
            break;
        };

        match fig_type {
            0 => parse_fig_type_00(fig_data, events),
            1 => parse_fig_type_01(fig_data, events),
            _ => {
                tracing::debug!(fig_type, "unknown FIG type");
            }
        }
    }
}

fn parse_fig_type_00(fig_data: &[u8], events: &mut Vec<FigEvent>) {
    let Some(&first) = fig_data.first() else {
        return;
    };
    let header = Fig00Header::parse(first);
    let extension = first & 0x1F;

    match extension {
        0 => events.extend(
            fig_00::parse_fig_00_00(fig_data)
                .into_iter()
                .map(FigEvent::Fig0000),
        ),
        1 => events.extend(
            fig_00::parse_fig_00_01(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0001),
        ),
        2 => events.extend(
            fig_00::parse_fig_00_02(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0002),
        ),
        3 => events.extend(
            fig_00::parse_fig_00_03(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0003),
        ),
        4 => events.extend(
            fig_00::parse_fig_00_04(fig_data)
                .into_iter()
                .map(FigEvent::Fig0004),
        ),
        5 => events.extend(
            fig_00::parse_fig_00_05(fig_data)
                .into_iter()
                .map(FigEvent::Fig0005),
        ),
        6 => events.extend(
            fig_00::parse_fig_00_06(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0006),
        ),
        7 => events.extend(
            fig_00::parse_fig_00_07(fig_data)
                .into_iter()
                .map(FigEvent::Fig0007),
        ),
        8 => events.extend(
            fig_00::parse_fig_00_08(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0008),
        ),
        9 => events.extend(
            fig_00::parse_fig_00_09(fig_data)
                .into_iter()
                .map(FigEvent::Fig0009),
        ),
        10 => events.extend(
            fig_00::parse_fig_00_10(fig_data)
                .into_iter()
                .map(FigEvent::Fig0010),
        ),
        13 => events.extend(
            fig_00::parse_fig_00_13(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0013),
        ),
        14 => events.extend(
            fig_00::parse_fig_00_14(fig_data)
                .into_iter()
                .map(FigEvent::Fig0014),
        ),
        17 => events.extend(
            fig_00::parse_fig_00_17(fig_data)
                .into_iter()
                .map(FigEvent::Fig0017),
        ),
        18 => events.extend(
            fig_00::parse_fig_00_18(fig_data)
                .into_iter()
                .map(FigEvent::Fig0018),
        ),
        19 => events.extend(
            fig_00::parse_fig_00_19(fig_data)
                .into_iter()
                .map(FigEvent::Fig0019),
        ),
        20 => events.extend(
            fig_00::parse_fig_00_20(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0020),
        ),
        21 => events.extend(
            fig_00::parse_fig_00_21(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0021),
        ),
        24 => events.extend(
            fig_00::parse_fig_00_24(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0024),
        ),
        25 => events.extend(
            fig_00::parse_fig_00_25(fig_data)
                .into_iter()
                .map(FigEvent::Fig0025),
        ),
        26 => events.extend(
            fig_00::parse_fig_00_26(fig_data)
                .into_iter()
                .map(FigEvent::Fig0026),
        ),
        other => tracing::debug!(extension = other, "unknown FIG 0/x extension"),
    }
}

fn parse_fig_type_01(fig_data: &[u8], events: &mut Vec<FigEvent>) {
    let Some(&first) = fig_data.first() else {
        return;
    };
    let header = Fig01Header::parse(first);
    let extension = first & 0x07;

    match extension {
        0 => events.extend(
            fig_01::parse_fig_01_00(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0100),
        ),
        1 => events.extend(
            fig_01::parse_fig_01_01(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0101),
        ),
        4 => events.extend(
            fig_01::parse_fig_01_04(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0104),
        ),
        5 => events.extend(
            fig_01::parse_fig_01_05(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0105),
        ),
        6 => events.extend(
            fig_01::parse_fig_01_06(fig_data, header)
                .into_iter()
                .map(FigEvent::Fig0106),
        ),
        other => tracing::debug!(extension = other, "unknown FIG 1/x extension"),
    }
}
