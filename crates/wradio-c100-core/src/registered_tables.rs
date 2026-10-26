//! Registered-value tables referenced by several FIG parsers: character
//! sets, language codes, data service component types, user application
//! types, programme types, and announcement types.

/// Character set identifier carried by FIG type 1 labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CharacterSet {
    /// Code 0.
    EbuLatin,
    /// Code 6.
    Ucs2,
    /// Code 15.
    Utf8,
    Other(u8),
}

impl CharacterSet {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::EbuLatin,
            6 => Self::Ucs2,
            15 => Self::Utf8,
            other => Self::Other(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::EbuLatin => "EBU Latin",
            Self::Ucs2 => "UCS-2",
            Self::Utf8 => "UTF-8",
            Self::Other(_) => "",
        }
    }
}

/// Maximum valid language code index.
pub const LANGUAGE_MAX: u8 = 0x7F;

/// Language names, indexed by the 8-bit language code from FIG 0/5.
///
/// The reference table (`registered_tables.h`) has 127 entries (indices
/// 0x00-0x7E) despite `LANGUAGE_MAX` being 0x7F -- an off-by-one already
/// present upstream. [`language_name`] falls back to index 0 for any code
/// outside this array rather than indexing out of bounds.
pub const LANGUAGE: [&str; 127] = [
    "Unknown/not applicable",
    "Albanian",
    "Breton",
    "Catalan",
    "Croatian",
    "Welsh",
    "Czech",
    "Danish",
    "German",
    "English",
    "Spanish",
    "Esperanto",
    "Estonian",
    "Basque",
    "Faroese",
    "French",
    "Frisian",
    "Irish",
    "Gaelic",
    "Galician",
    "Icelandic",
    "Italian",
    "Lappish",
    "Latin",
    "Latvian",
    "Luxembourgian",
    "Lithuanian",
    "Hungarian",
    "Maltese",
    "Dutch",
    "Norwegian",
    "Occitan",
    "Polish",
    "Portuguese",
    "Romanian",
    "Romansh",
    "Serbian",
    "Slovak",
    "Slovene",
    "Finnish",
    "Swedish",
    "Turkish",
    "Flemish",
    "Walloon",
    "rfu_2C",
    "rfu_2D",
    "rfu_2E",
    "rfu_2F",
    "National_Assignemnt_30",
    "National_Assignemnt_31",
    "National_Assignemnt_32",
    "National_Assignemnt_33",
    "National_Assignemnt_34",
    "National_Assignemnt_35",
    "National_Assignemnt_36",
    "National_Assignemnt_37",
    "National_Assignemnt_38",
    "National_Assignemnt_39",
    "National_Assignemnt_3A",
    "National_Assignemnt_3B",
    "National_Assignemnt_3C",
    "National_Assignemnt_3D",
    "National_Assignemnt_3F",
    "Background sound/clean feed",
    "rfu_41",
    "rfu_42",
    "rfu_43",
    "rfu_44",
    "Zulu",
    "Vietnamese",
    "Uzbek",
    "Urdu",
    "Ukranian",
    "Thai",
    "Telugu",
    "Tatar",
    "Tamil",
    "Tadzhik",
    "Swahili",
    "Sranan Tongo",
    "Somali",
    "Sinhalese",
    "Shona",
    "Serbo-Croat",
    "Rusyn",
    "Russian",
    "Quechua",
    "Pushtu",
    "Punjabi",
    "Persian",
    "Papiamento",
    "Oriya",
    "Nepali",
    "Ndebele",
    "Marathi",
    "Moldavian",
    "Malaysian",
    "Malagasay",
    "Macedonian",
    "Laotian",
    "Korean",
    "Khmer",
    "Kazakh",
    "Kannada",
    "Japanese",
    "Indonesian",
    "Hindi",
    "Hebrew",
    "Hausa",
    "Gurani",
    "Gujurati",
    "Greek",
    "Georgian",
    "Fulani",
    "Dari",
    "Chuvash",
    "Chinese",
    "Burmese",
    "Bulgarian",
    "Bengali",
    "Belorussian",
    "Bambora",
    "Azerbaijani",
    "Assamese",
    "Armenian",
    "Arabic",
    "Amharic",
];

/// Looks up a language name, falling back to index 0 ("Unknown/not
/// applicable") for an out-of-range code.
pub fn language_name(code: u8) -> &'static str {
    LANGUAGE.get(code as usize).copied().unwrap_or(LANGUAGE[0])
}

/// Data service component type, from FIG 0/3 / FIG 0/13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataServiceComponentType {
    UnspecifiedData,
    TrafficMessageChannel,
    EmergencyWarningSystem,
    InteractiveTextTransmissionSystem,
    Paging,
    TransparentDataChannel,
    Mpeg2TransportStream,
    EmbeddedIpPackets,
    MultimediaObjectTransfer,
    ProprietaryService,
    NotUsed(u8),
    Reserved(u8),
}

impl DataServiceComponentType {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::UnspecifiedData,
            1 => Self::TrafficMessageChannel,
            2 => Self::EmergencyWarningSystem,
            3 => Self::InteractiveTextTransmissionSystem,
            4 => Self::Paging,
            5 => Self::TransparentDataChannel,
            24 => Self::Mpeg2TransportStream,
            59 => Self::EmbeddedIpPackets,
            60 => Self::MultimediaObjectTransfer,
            61 => Self::ProprietaryService,
            62 | 63 => Self::NotUsed(code),
            other => Self::Reserved(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::UnspecifiedData => "Unspecified data",
            Self::TrafficMessageChannel => "Traffic Message Channel (TMC)",
            Self::EmergencyWarningSystem => "Emergency Warning System (EWS)",
            Self::InteractiveTextTransmissionSystem => {
                "Interactive Text Transmission System (ITTS)"
            }
            Self::Paging => "Paging",
            Self::TransparentDataChannel => "Transparent Data Channel (TDC)",
            Self::Mpeg2TransportStream => "MPEG-2 Transport Stream",
            Self::EmbeddedIpPackets => "Embedded IP packets",
            Self::MultimediaObjectTransfer => "Multimedia Object Transfer (MOT)",
            Self::ProprietaryService => "Proprietary Service",
            Self::NotUsed(_) => "Not used",
            Self::Reserved(_) => "",
        }
    }
}

/// X-PAD / MSC data-group user application type, from FIG 0/13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserApplicationType {
    Rfu,
    DynamicLabel,
    MotSlideshow,
    MotBroadcastWebsite,
    Tpeg,
    Dgps,
    Tmc,
    Epg,
    DabJava,
    Dmb,
    IpdcServices,
    VoiceApplications,
    Middleware,
    Filecasting,
    Journaline,
    Other(u16),
}

impl UserApplicationType {
    pub fn from_code(code: u16) -> Self {
        match code {
            0 => Self::Rfu,
            1 => Self::DynamicLabel,
            2 => Self::MotSlideshow,
            3 => Self::MotBroadcastWebsite,
            4 => Self::Tpeg,
            5 => Self::Dgps,
            6 => Self::Tmc,
            7 => Self::Epg,
            8 => Self::DabJava,
            9 => Self::Dmb,
            10 => Self::IpdcServices,
            11 => Self::VoiceApplications,
            12 => Self::Middleware,
            13 => Self::Filecasting,
            0x044A => Self::Journaline,
            other => Self::Other(other),
        }
    }
}

/// Programme Type names: `[full name, 16-char abbreviation, 8-char
/// abbreviation]`, indexed by the FIG 0/17 `intPtyCode`.
///
/// The upstream table has a missing comma in the "Documentary" row
/// (`{"Documentary", "Documentary" "Document"}`), which C++ string-literal
/// concatenation silently turns into a 2-element initializer
/// (`"DocumentaryDocument"`) for a 3-element row, leaving the 8-char
/// abbreviation empty. Fixed here to the evidently intended three separate
/// strings.
pub const PROGRAMME_TYPE_NAME: [[&str; 3]; 32] = [
    ["No programme type", "None", "None"],
    ["News", "News", "News"],
    ["Current Affairs", "Current_Affairs", "Affairs"],
    ["Information", "Information", "Info"],
    ["Sport", "Sport", "Sport"],
    ["Education", "Education", "Educate"],
    ["Drama", "Drama", "Drama"],
    ["Culture", "Arts", "Arts"],
    ["Science", "Science", "Science"],
    ["Varied", "Talk", "Talk"],
    ["Pop Music", "Pop_Music", "Pop"],
    ["Rock Music", "Rock_Music", "Rock"],
    ["Easy Listening Music", "Easy_Listening", "Easy"],
    ["Light Classical", "Light_Classical", "Classics"],
    ["Serious Classical", "Classical_Music", "Classics"],
    ["Other Music", "Other_Music", "Other_M"],
    ["Weather/Meteorology", "Weather", "Weather"],
    ["Finance/Business", "Finance", "Finance"],
    ["Children's programmes", "Children's", "Children"],
    ["Social Affairs", "Factual", "Factual"],
    ["Religion", "Religion", "Religion"],
    ["Phone In", "Phone_In", "Phone_In"],
    ["Travel", "Travel", "Travel"],
    ["Leisure", "Leisure", "Leisure"],
    ["Jazz Music", "Jazz_and_Blues", "Jazz"],
    ["Country Music", "Country_Music", "Country"],
    ["National Music", "National_Music", "Nation_M"],
    ["Oldies Music", "Oldies_Music", "Oldies"],
    ["Folk Music", "Folk_Music", "Folk"],
    ["Documentary", "Documentary", "Document"],
    ["Not used", "Not_Used", "Not_Used"],
    ["Not used", "Not_Used", "Not_Used"],
];

/// Announcement type, from FIG 0/18/0/19/0/25/0/26.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnouncementType {
    Alarm,
    RoadTrafficFlash,
    TransportFlash,
    WarningService,
    NewsFlash,
    AreaWeatherFlash,
    EventAnnouncement,
    SpecialEvent,
    ProgrammeInformation,
    SportReport,
    FinancialReport,
    Reserved(u8),
}

impl AnnouncementType {
    pub fn from_bit(bit: u8) -> Self {
        match bit {
            0 => Self::Alarm,
            1 => Self::RoadTrafficFlash,
            2 => Self::TransportFlash,
            3 => Self::WarningService,
            4 => Self::NewsFlash,
            5 => Self::AreaWeatherFlash,
            6 => Self::EventAnnouncement,
            7 => Self::SpecialEvent,
            8 => Self::ProgrammeInformation,
            9 => Self::SportReport,
            10 => Self::FinancialReport,
            other => Self::Reserved(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Alarm => "Alarm",
            Self::RoadTrafficFlash => "Road Traffic flash",
            Self::TransportFlash => "Transport flash",
            Self::WarningService => "Warning/Service",
            Self::NewsFlash => "News flash",
            Self::AreaWeatherFlash => "Area weather flash",
            Self::EventAnnouncement => "Event announcement",
            Self::SpecialEvent => "Special event",
            Self::ProgrammeInformation => "Programme Information",
            Self::SportReport => "Sport report",
            Self::FinancialReport => "Financial report",
            Self::Reserved(_) => "Reserved",
        }
    }
}
