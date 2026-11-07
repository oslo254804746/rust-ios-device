//! IMG4 component personalization and ASN.1 DER envelope stitching.
//!
//! Aligned with upstream `pymobiledevice3/restore/img4.py`:
//! - Maps IPSW `BuildManifest` component names to canonical 4-byte FourCC tags (`COMPONENT_FOURCC`).
//! - Parses existing DER `IM4P` payloads to rewrite their 4-byte tag or wraps raw firmware payloads
//!   into canonical `IM4P` ASN.1 structures.
//! - Constructs optional `IM4R` restore property bags (`BNCN` boot nonce, `snid`, `anid`, and
//!   `<Component>-TBM` entries such as `ucon` / `ucer` from TSS responses).
//! - Stitches the final `IMG4` DER envelope:
//!   `SEQUENCE { IA5String("IMG4"), IM4P, [0] EXPLICIT IM4M, [1] EXPLICIT IM4R OPTIONAL }`.

use super::RestoreError;

/// Canonical mapping from `BuildManifest` component names to 4-byte IM4P FourCC tags.
///
/// Mirrors `COMPONENT_FOURCC` in `pymobiledevice3/restore/img4.py`.
pub const COMPONENT_FOURCC: &[(&str, [u8; 4])] = &[
    ("ACIBT", *b"acib"),
    ("ADRT", *b"adrt"),
    ("ANS", *b"ansf"),
    ("AOP", *b"aopf"),
    ("AVE", *b"avef"),
    ("Alamo", *b"almo"),
    ("Ap,AMPR", *b"ampr"),
    ("Ap,ANE1", *b"ane1"),
    ("Ap,ANE2", *b"ane2"),
    ("Ap,ANE3", *b"ane3"),
    ("Ap,AudioBootChime", *b"aupr"),
    ("Ap,AudioPowerAttachChime", *b"auac"),
    ("Ap,BootabilityBrainTrustCache", *b"trbb"),
    ("Ap,CIO", *b"ciof"),
    ("Ap,DCP2", *b"dcp2"),
    ("Ap,DarBoot", *b"darb"),
    ("Ap,Engine", *b"eg0n"),
    ("Ap,Engine1", *b"eg1n"),
    ("Ap,Engine3", *b"eg3n"),
    ("Ap,ExclaveOS", *b"exos"),
    ("Ap,ExclaveOSIntegrityCatalog", *b"exic"),
    ("Ap,ExclaveOSStaticTrustCache", *b"extc"),
    ("Ap,GAE1", *b"gae1"),
    ("Ap,HapticAssets", *b"hpas"),
    ("Ap,LocalBoot", *b"lobo"),
    ("Ap,LocalPolicy", *b"lpol"),
    ("Ap,Momentum", *b"mmtm"),
    ("Ap,NextStageIM4MHash", *b"nsih"),
    ("Ap,OS", *b"rosi"),
    ("Ap,OSRamdisk", *b"osrd"),
    ("Ap,OverridableTrustCache", *b"ovtc"),
    ("Ap,PlaygroundsAssets", *b"pgas"),
    ("Ap,PMP2", *b"pmp2"),
    ("Ap,RestoreANE1", *b"ran1"),
    ("Ap,RestoreANE2", *b"ran2"),
    ("Ap,RestoreANE3", *b"ran3"),
    ("Ap,RestoreCIO", *b"rcio"),
    ("Ap,RestoreDCP2", *b"rdc2"),
    ("Ap,RestoreDarBoot", *b"rdrb"),
    ("Ap,RestoreExclaveOS", *b"rxos"),
    ("Ap,RestoreExclaveOSIntegrityCatalog", *b"rxic"),
    ("Ap,RestoreExclaveOSStaticTrustCache", *b"rxtc"),
    ("Ap,RestoreGAE1", *b"rge1"),
    ("Ap,RestoreNVRam", *b"rnvr"),
    ("Ap,RestoreOverridableTrustCache", *b"rotc"),
    ("Ap,RestorePMP2", *b"rpm2"),
    ("Ap,RestorePrologue", *b"rplg"),
    ("Ap,RestoreSecureM3Firmware", *b"rsm3"),
    ("Ap,RestoreSecurePageTableMonitor", *b"rspt"),
    ("Ap,RestoreTMU", *b"rtmu"),
    ("Ap,RestoreTrustedExecutionMonitor", *b"rtrx"),
    ("Ap,RestorecL4", *b"rxcl"),
    ("Ap,Scorpius", *b"scpf"),
    ("Ap,SecureM3Firmware", *b"sm3f"),
    ("Ap,SecurePageTableMonitor", *b"sptm"),
    ("Ap,StickerAssets", *b"stas"),
    ("Ap,SystemVolumeCanonicalMetadata", *b"msys"),
    ("Ap,TMU", *b"tmuf"),
    ("Ap,TrustedExecutionMonitor", *b"trxm"),
    ("Ap,WatchfaceAssets", *b"wfas"),
    ("Ap,cL4", *b"cl4f"),
    ("AudioCodecFirmware", *b"acfw"),
    ("BatteryCharging", *b"glyC"),
    ("BatteryCharging0", *b"chg0"),
    ("BatteryCharging1", *b"chg1"),
    ("BatteryFull", *b"batF"),
    ("BatteryLow0", *b"bat0"),
    ("BatteryLow1", *b"bat1"),
    ("BatteryPlugin", *b"glyP"),
    ("CFELoader", *b"cfel"),
    ("CrownFirmware", *b"crwn"),
    ("DALI", *b"dali"),
    ("DCP", *b"dcpf"),
    ("DeviceTree", *b"dtre"),
    ("Diags", *b"diag"),
    ("EngineeringTrustCache", *b"dtrs"),
    ("ExtDCP", *b"edcp"),
    ("FTAP", *b"ftap"),
    ("FTAB", *b"ftab"),
    ("FTA2", *b"fta2"),
    ("FTSP", *b"ftsp"),
    ("GFX", *b"gfxf"),
    ("Hamm", *b"hamf"),
    ("Homer", *b"homr"),
    ("IOServiceXHCI", *b"xhci"),
    ("ISP", *b"ispf"),
    ("InputDevice", *b"ipdf"),
    ("KernelCache", *b"krnl"),
    ("LLB", *b"illb"),
    ("LeapHaptics", *b"lphp"),
    ("Liquid", *b"liqd"),
    ("LoadableTrustCache", *b"ltrs"),
    ("LowPowerWallet0", *b"lpw0"),
    ("LowPowerWallet1", *b"lpw1"),
    ("LowPowerWallet2", *b"lpw2"),
    ("MacEFI", *b"mefi"),
    ("MtpFirmware", *b"mtpf"),
    ("Multitouch", *b"mtfw"),
    ("NeedService", *b"nsrv"),
    ("OS", *b"OS\0\0"),
    ("OSRamdisk", *b"osrd"),
    ("PEHammer", *b"hmmr"),
    ("PERTOS", *b"pert"),
    ("PHLEET", *b"phlt"),
    ("PMP", *b"pmpf"),
    ("PersonalizedDMG", *b"pdmg"),
    ("RBM", *b"rmbt"),
    ("RTP", *b"rtpf"),
    ("Rap,AudioDSP", *b"radp"),
    ("Rap,BasebandBoot", *b"rbbb"),
    ("Rap,RTKitOS", *b"rkos"),
    ("Rap,RestoreRTKitOS", *b"rrko"),
    ("Rap,SoftwareBinaryDsp1", *b"sbd1"),
    ("Rap,SoftwareBinaryDsp2", *b"sbd2"),
    ("RecoveryMode", *b"recm"),
    ("RestoreANS", *b"rans"),
    ("RestoreAOP", *b"raop"),
    ("RestoreAVE", *b"rave"),
    ("RestoreDCP", *b"rdcp"),
    ("RestoreDeviceTree", *b"rdtr"),
    ("RestoreExtDCP", *b"recp"),
    ("RestoreGFX", *b"rgfx"),
    ("RestoreHapticAssets", *b"rhpa"),
    ("RestoreISP", *b"risp"),
    ("RestoreKernelCache", *b"rkrn"),
    ("RestoreLogo", *b"rlgo"),
    ("RestoreMtpFirmware", *b"rmtp"),
    ("RestoreMultitouch", *b"rmtf"),
    ("RestorePMP", *b"rpmp"),
    ("RestoreRamDisk", *b"rdsk"),
    ("RestoreSEP", *b"rsep"),
    ("RestoreSIO", *b"rsio"),
    ("RestoreSWDH", *b"rswd"),
    ("RestoreTrustCache", *b"rtsc"),
    ("SCE", *b"scef"),
    ("SCE1Firmware", *b"sc1f"),
    ("SEP", *b"sepi"),
    ("SIO", *b"siof"),
    ("SWDH", *b"swdh"),
    ("SmartIOFirmware", *b"siof"),
    ("StaticTrustCache", *b"trst"),
    ("SystemLocker", *b"lckr"),
    ("SystemVolume", *b"isys"),
    ("Timer", *b"timr"),
    ("TouchCache", *b"tcch"),
    ("WCHFirmwareUpdate", *b"wchf"),
    ("iBEC", *b"ibec"),
    ("iBSS", *b"ibss"),
    ("iBoot", *b"ibot"),
    ("iBootData", *b"ibdt"),
    ("iBootTest", *b"itst"),
    ("rFTA", *b"rfta"),
    ("rFTS", *b"rfts"),
];

/// Look up the 4-byte IM4P FourCC tag for a `BuildManifest` component name.
pub fn component_fourcc(name: &str) -> Option<[u8; 4]> {
    COMPONENT_FOURCC
        .iter()
        .find_map(|(candidate, tag)| (*candidate == name).then_some(*tag))
}

/// Value stored inside an `IM4R` restore property (`BNCN`, `snid`, `anid`, `ucon`, `ucer`, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestorePropertyValue {
    Uint64(u64),
    Bytes(Vec<u8>),
}

/// Single tagged property entry inside an `IM4R` restore property bag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreProperty {
    pub fourcc: [u8; 4],
    pub value: RestorePropertyValue,
}

impl RestoreProperty {
    pub fn fourcc_str(&self) -> String {
        String::from_utf8_lossy(&self.fourcc).into_owned()
    }
}

/// Personalized and stitched `IMG4` component ready for Recovery iBoot or `restored`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StitchedImg4Component {
    pub component_name: String,
    pub fourcc: [u8; 4],
    pub description: String,
    pub im4p_bytes: Vec<u8>,
    pub im4m_bytes: Vec<u8>,
    pub im4r_bytes: Option<Vec<u8>>,
    pub restore_properties: Vec<RestoreProperty>,
    pub img4_bytes: Vec<u8>,
}

impl StitchedImg4Component {
    pub fn fourcc_str(&self) -> String {
        String::from_utf8_lossy(&self.fourcc).into_owned()
    }
}

/// Decoded representation of a stitched `IMG4` DER envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImg4Envelope {
    pub fourcc: [u8; 4],
    pub description: String,
    pub payload: Vec<u8>,
    pub im4m_ticket: Vec<u8>,
    pub im4r_raw: Option<Vec<u8>>,
}

const DER_SEQUENCE: u8 = 0x30;
const DER_SET: u8 = 0x31;
const DER_INTEGER: u8 = 0x02;
const DER_OCTET_STRING: u8 = 0x04;
const DER_IA5_STRING: u8 = 0x16;
const DER_CONTEXT_CONSTRUCTED_0: u8 = 0xA0;
const DER_CONTEXT_CONSTRUCTED_1: u8 = 0xA1;

fn encode_der_length(len: usize, out: &mut Vec<u8>) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let mut bytes = Vec::new();
        let mut n = len;
        while n > 0 {
            bytes.push((n & 0xff) as u8);
            n >>= 8;
        }
        bytes.reverse();
        out.push(0x80 | (bytes.len() as u8));
        out.extend_from_slice(&bytes);
    }
}

fn encode_der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 5 + content.len());
    out.push(tag);
    encode_der_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

fn encode_der_ia5_string(text: &[u8]) -> Vec<u8> {
    encode_der_tlv(DER_IA5_STRING, text)
}

fn encode_der_octet_string(data: &[u8]) -> Vec<u8> {
    encode_der_tlv(DER_OCTET_STRING, data)
}

fn encode_der_uint64(value: u64) -> Vec<u8> {
    let be = value.to_be_bytes();
    let first_non_zero = be
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(be.len().saturating_sub(1));
    let slice = &be[first_non_zero..];
    let mut content = Vec::with_capacity(slice.len() + 1);
    if slice[0] & 0x80 != 0 {
        content.push(0x00);
    }
    content.extend_from_slice(slice);
    encode_der_tlv(DER_INTEGER, &content)
}

fn decode_der_tlv(input: &[u8]) -> Result<(u8, &[u8], &[u8]), RestoreError> {
    if input.len() < 2 {
        return Err(RestoreError::Protocol("truncated DER TLV header".into()));
    }
    let tag = input[0];
    let len_byte = input[1];
    let (header_len, content_len) = if len_byte & 0x80 == 0 {
        (2usize, len_byte as usize)
    } else {
        let count = (len_byte & 0x7f) as usize;
        if count == 0 || count > 4 || input.len() < 2 + count {
            return Err(RestoreError::Protocol("invalid DER length encoding".into()));
        }
        let mut len = 0usize;
        for &b in &input[2..2 + count] {
            len = (len << 8) | (b as usize);
        }
        (2 + count, len)
    };
    let end = header_len
        .checked_add(content_len)
        .filter(|&end| end <= input.len())
        .ok_or_else(|| RestoreError::Protocol("DER TLV length exceeds buffer".into()))?;
    Ok((tag, &input[header_len..end], &input[end..]))
}

/// Construct a canonical DER `IM4P` payload buffer.
pub fn build_im4p(fourcc: [u8; 4], description: &str, payload: &[u8]) -> Vec<u8> {
    let mut seq = Vec::new();
    seq.extend_from_slice(&encode_der_ia5_string(b"IM4P"));
    seq.extend_from_slice(&encode_der_ia5_string(&fourcc));
    seq.extend_from_slice(&encode_der_ia5_string(description.as_bytes()));
    seq.extend_from_slice(&encode_der_octet_string(payload));
    encode_der_tlv(DER_SEQUENCE, &seq)
}

/// Parse an existing `IM4P` DER structure and rewrite its 4-byte FourCC tag, or wrap a raw payload
/// into a canonical `IM4P` DER structure when the input is not already an `IM4P` sequence.
pub fn parse_or_rewrite_im4p(
    component_name: &str,
    raw_or_im4p: &[u8],
    fourcc_override: Option<[u8; 4]>,
) -> Result<(Vec<u8>, [u8; 4], String), RestoreError> {
    if raw_or_im4p.is_empty() {
        return Err(RestoreError::Protocol(format!(
            "component {component_name} payload is empty"
        )));
    }
    let resolved_fourcc = fourcc_override
        .or_else(|| component_fourcc(component_name))
        .ok_or_else(|| {
            RestoreError::Protocol(format!(
                "unknown IMG4 component fourcc for {component_name}"
            ))
        })?;

    if let Ok((DER_SEQUENCE, seq_body, trailing)) = decode_der_tlv(raw_or_im4p) {
        if trailing.is_empty() {
            if let Ok((DER_IA5_STRING, magic, rest1)) = decode_der_tlv(seq_body) {
                if magic == b"IM4P" {
                    let (tag_fourcc, _old_fourcc, rest2) = decode_der_tlv(rest1)?;
                    if tag_fourcc != DER_IA5_STRING {
                        return Err(RestoreError::Protocol(
                            "IM4P second element must be IA5String fourcc".into(),
                        ));
                    }
                    let (tag_desc, desc_bytes, rest3) = decode_der_tlv(rest2)?;
                    if tag_desc != DER_IA5_STRING {
                        return Err(RestoreError::Protocol(
                            "IM4P third element must be IA5String description".into(),
                        ));
                    }
                    let (tag_data, payload_bytes, rest4) = decode_der_tlv(rest3)?;
                    if tag_data != DER_OCTET_STRING {
                        return Err(RestoreError::Protocol(
                            "IM4P fourth element must be OCTET STRING data".into(),
                        ));
                    }
                    let description = String::from_utf8_lossy(desc_bytes).into_owned();
                    let mut rebuilt = Vec::new();
                    rebuilt.extend_from_slice(&encode_der_ia5_string(b"IM4P"));
                    rebuilt.extend_from_slice(&encode_der_ia5_string(&resolved_fourcc));
                    rebuilt.extend_from_slice(&encode_der_ia5_string(desc_bytes));
                    rebuilt.extend_from_slice(&encode_der_octet_string(payload_bytes));
                    rebuilt.extend_from_slice(rest4);
                    return Ok((
                        encode_der_tlv(DER_SEQUENCE, &rebuilt),
                        resolved_fourcc,
                        description,
                    ));
                }
            }
        }
    }

    let description = format!("rust-ios-device:{component_name}");
    let im4p = build_im4p(resolved_fourcc, &description, raw_or_im4p);
    Ok((im4p, resolved_fourcc, description))
}

fn encode_restore_property(prop: &RestoreProperty) -> Vec<u8> {
    // Private constructed tag (0xE0 | ...) or universal SEQUENCE inside SET:
    // Mirrors pyasn1 RestoreProperty: SEQUENCE { IA5String(fourcc), value } wrapped in explicit tag
    let mut inner = Vec::new();
    inner.extend_from_slice(&encode_der_ia5_string(&prop.fourcc));
    match &prop.value {
        RestorePropertyValue::Uint64(v) => inner.extend_from_slice(&encode_der_uint64(*v)),
        RestorePropertyValue::Bytes(b) => inner.extend_from_slice(&encode_der_octet_string(b)),
    }
    encode_der_tlv(DER_SEQUENCE, &inner)
}

/// Build an `IM4R` DER sequence from an optional `BNCN` boot nonce and additional restore properties.
pub fn build_im4r(boot_nonce: Option<&[u8]>, properties: &[RestoreProperty]) -> Vec<u8> {
    let mut all_props = Vec::new();
    if let Some(nonce) = boot_nonce {
        all_props.push(RestoreProperty {
            fourcc: *b"BNCN",
            value: RestorePropertyValue::Bytes(nonce.to_vec()),
        });
    }
    all_props.extend_from_slice(properties);

    let mut set_content = Vec::new();
    for prop in &all_props {
        set_content.extend_from_slice(&encode_restore_property(prop));
    }
    let prop_set = encode_der_tlv(DER_SET, &set_content);

    let mut seq = Vec::new();
    seq.extend_from_slice(&encode_der_ia5_string(b"IM4R"));
    seq.extend_from_slice(&prop_set);
    encode_der_tlv(DER_SEQUENCE, &seq)
}

/// Extract `<component_name>-TBM` and `snid`/`anid` restore properties from a TSS response dictionary.
///
/// Mirrors `construct_restore_properties` in `pymobiledevice3/restore/img4.py`.
pub fn extract_tss_restore_properties(
    component_name: &str,
    tss_response: &plist::Dictionary,
) -> Result<Vec<RestoreProperty>, RestoreError> {
    let mut props = Vec::new();

    for (manifest_key, tag) in [
        ("Ap,SystemikaFuseID", *b"snid"),
        ("Ap,AnalyticsFuseID", *b"anid"),
    ] {
        if let Some(val) = tss_response.get(manifest_key) {
            if let Some(n) = val.as_unsigned_integer() {
                props.push(RestoreProperty {
                    fourcc: tag,
                    value: RestorePropertyValue::Uint64(n),
                });
            } else if let Some(n) = val.as_signed_integer().filter(|&n| n >= 0) {
                props.push(RestoreProperty {
                    fourcc: tag,
                    value: RestorePropertyValue::Uint64(n as u64),
                });
            }
        }
    }

    let tbm_key = format!("{component_name}-TBM");
    if let Some(tbm_dict) = tss_response
        .get(&tbm_key)
        .and_then(plist::Value::as_dictionary)
    {
        for (key, value) in tbm_dict {
            let bytes = key.as_bytes();
            if bytes.len() != 4 {
                return Err(RestoreError::Protocol(format!(
                    "invalid 4-char TBM property key '{key}' in {tbm_key}"
                )));
            }
            let fourcc = [bytes[0], bytes[1], bytes[2], bytes[3]];
            match value {
                plist::Value::Data(data) => props.push(RestoreProperty {
                    fourcc,
                    value: RestorePropertyValue::Bytes(data.clone()),
                }),
                plist::Value::Integer(int) => {
                    let v = int
                        .as_unsigned()
                        .or_else(|| int.as_signed().filter(|&n| n >= 0).map(|n| n as u64))
                        .ok_or_else(|| {
                            RestoreError::Protocol(format!(
                                "negative TBM integer for '{key}' in {tbm_key}"
                            ))
                        })?;
                    props.push(RestoreProperty {
                        fourcc,
                        value: RestorePropertyValue::Uint64(v),
                    });
                }
                _ => {
                    return Err(RestoreError::Protocol(format!(
                        "unsupported TBM value type for '{key}' in {tbm_key}"
                    )))
                }
            }
        }
    }

    Ok(props)
}

/// Stitch an `IM4P` (or raw component payload), an `ApImg4Ticket` (`IM4M`), and optional `IM4R`
/// restore properties into a complete `IMG4` DER binary.
///
/// Mirrors `stitch_component` in `pymobiledevice3/restore/img4.py`.
pub fn stitch_img4_component(
    component_name: &str,
    raw_or_im4p: &[u8],
    ap_img4_ticket: &[u8],
    tss_response: Option<&plist::Dictionary>,
    boot_nonce: Option<&[u8]>,
    fourcc_override: Option<[u8; 4]>,
) -> Result<StitchedImg4Component, RestoreError> {
    if ap_img4_ticket.is_empty() {
        return Err(RestoreError::Protocol(format!(
            "cannot stitch IMG4 component {component_name} with empty ApImg4Ticket"
        )));
    }
    let (im4p_bytes, fourcc, description) =
        parse_or_rewrite_im4p(component_name, raw_or_im4p, fourcc_override)?;

    let mut restore_properties = match tss_response {
        Some(tss) => extract_tss_restore_properties(component_name, tss)?,
        None => Vec::new(),
    };

    let im4r_bytes = if boot_nonce.is_some() || !restore_properties.is_empty() {
        let im4r = build_im4r(boot_nonce, &restore_properties);
        if let Some(nonce) = boot_nonce {
            let mut with_nonce = vec![RestoreProperty {
                fourcc: *b"BNCN",
                value: RestorePropertyValue::Bytes(nonce.to_vec()),
            }];
            with_nonce.append(&mut restore_properties);
            restore_properties = with_nonce;
        }
        Some(im4r)
    } else {
        None
    };

    let mut img4_seq = Vec::new();
    img4_seq.extend_from_slice(&encode_der_ia5_string(b"IMG4"));
    img4_seq.extend_from_slice(&im4p_bytes);
    img4_seq.extend_from_slice(&encode_der_tlv(DER_CONTEXT_CONSTRUCTED_0, ap_img4_ticket));
    if let Some(ref im4r) = im4r_bytes {
        img4_seq.extend_from_slice(&encode_der_tlv(DER_CONTEXT_CONSTRUCTED_1, im4r));
    }
    let img4_bytes = encode_der_tlv(DER_SEQUENCE, &img4_seq);

    Ok(StitchedImg4Component {
        component_name: component_name.to_owned(),
        fourcc,
        description,
        im4p_bytes,
        im4m_bytes: ap_img4_ticket.to_vec(),
        im4r_bytes,
        restore_properties,
        img4_bytes,
    })
}

/// Parse and verify a stitched `IMG4` DER envelope.
pub fn parse_img4_envelope(bytes: &[u8]) -> Result<ParsedImg4Envelope, RestoreError> {
    let (tag_seq, seq_body, trailing) = decode_der_tlv(bytes)?;
    if tag_seq != DER_SEQUENCE || !trailing.is_empty() {
        return Err(RestoreError::Protocol(
            "IMG4 envelope must be a single DER SEQUENCE".into(),
        ));
    }
    let (tag_magic, magic, rest1) = decode_der_tlv(seq_body)?;
    if tag_magic != DER_IA5_STRING || magic != b"IMG4" {
        return Err(RestoreError::Protocol(
            "IMG4 envelope missing IA5String(\"IMG4\")".into(),
        ));
    }
    let (tag_im4p, im4p_body, rest2) = decode_der_tlv(rest1)?;
    if tag_im4p != DER_SEQUENCE {
        return Err(RestoreError::Protocol(
            "IMG4 second element must be IM4P SEQUENCE".into(),
        ));
    }
    let (tag_im4p_magic, im4p_magic, im4p_rest1) = decode_der_tlv(im4p_body)?;
    if tag_im4p_magic != DER_IA5_STRING || im4p_magic != b"IM4P" {
        return Err(RestoreError::Protocol(
            "IM4P missing IA5String(\"IM4P\")".into(),
        ));
    }
    let (tag_fourcc, fourcc_bytes, im4p_rest2) = decode_der_tlv(im4p_rest1)?;
    if tag_fourcc != DER_IA5_STRING || fourcc_bytes.len() != 4 {
        return Err(RestoreError::Protocol(
            "IM4P fourcc must be 4-byte IA5String".into(),
        ));
    }
    let fourcc = [
        fourcc_bytes[0],
        fourcc_bytes[1],
        fourcc_bytes[2],
        fourcc_bytes[3],
    ];
    let (tag_desc, desc_bytes, im4p_rest3) = decode_der_tlv(im4p_rest2)?;
    if tag_desc != DER_IA5_STRING {
        return Err(RestoreError::Protocol(
            "IM4P description must be IA5String".into(),
        ));
    }
    let (tag_data, payload_bytes, _) = decode_der_tlv(im4p_rest3)?;
    if tag_data != DER_OCTET_STRING {
        return Err(RestoreError::Protocol(
            "IM4P payload must be OCTET STRING".into(),
        ));
    }

    let (tag_im4m, im4m_ticket, rest3) = decode_der_tlv(rest2)?;
    if tag_im4m != DER_CONTEXT_CONSTRUCTED_0 || im4m_ticket.is_empty() {
        return Err(RestoreError::Protocol(
            "IMG4 missing [0] EXPLICIT IM4M ticket".into(),
        ));
    }

    let im4r_raw = if !rest3.is_empty() {
        let (tag_im4r, im4r_body, rest4) = decode_der_tlv(rest3)?;
        if tag_im4r != DER_CONTEXT_CONSTRUCTED_1 || !rest4.is_empty() {
            return Err(RestoreError::Protocol(
                "IMG4 optional fourth element must be [1] EXPLICIT IM4R".into(),
            ));
        }
        Some(im4r_body.to_vec())
    } else {
        None
    };

    Ok(ParsedImg4Envelope {
        fourcc,
        description: String::from_utf8_lossy(desc_bytes).into_owned(),
        payload: payload_bytes.to_vec(),
        im4m_ticket: im4m_ticket.to_vec(),
        im4r_raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_fourcc_table_matches_upstream_pymobiledevice3() {
        assert_eq!(component_fourcc("RestoreLogo"), Some(*b"rlgo"));
        assert_eq!(component_fourcc("RestoreRamDisk"), Some(*b"rdsk"));
        assert_eq!(component_fourcc("RestoreDeviceTree"), Some(*b"rdtr"));
        assert_eq!(component_fourcc("RestoreSEP"), Some(*b"rsep"));
        assert_eq!(component_fourcc("RestoreKernelCache"), Some(*b"rkrn"));
        assert_eq!(component_fourcc("KernelCache"), Some(*b"krnl"));
        assert_eq!(component_fourcc("DeviceTree"), Some(*b"dtre"));
        assert_eq!(component_fourcc("LLB"), Some(*b"illb"));
        assert_eq!(component_fourcc("iBoot"), Some(*b"ibot"));
        assert_eq!(component_fourcc("SEP"), Some(*b"sepi"));
        assert_eq!(component_fourcc("SystemVolume"), Some(*b"isys"));
        assert_eq!(
            component_fourcc("Ap,SystemVolumeCanonicalMetadata"),
            Some(*b"msys")
        );
        assert_eq!(component_fourcc("UnknownComponent"), None);
    }

    #[test]
    fn stitches_existing_im4p_and_patches_fourcc_with_tbm_properties() {
        let original_im4p = build_im4p(*b"krnl", "KernelCache-orig", b"KERNEL_PAYLOAD_BYTES");
        let tss = plist::Dictionary::from_iter([
            ("Ap,SystemikaFuseID", plist::Value::Integer(7.into())),
            (
                "RestoreKernelCache-TBM",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ucon", plist::Value::Data(vec![0x11, 0x22])),
                    ("ucer", plist::Value::Data(vec![0x33, 0x44])),
                ])),
            ),
        ]);

        let stitched = stitch_img4_component(
            "RestoreKernelCache",
            &original_im4p,
            b"FAKE_AP_IMG4_TICKET",
            Some(&tss),
            Some(&[0xAA, 0xBB, 0xCC]),
            None,
        )
        .unwrap();

        assert_eq!(&stitched.fourcc, b"rkrn");
        assert_eq!(stitched.description, "KernelCache-orig");
        assert_eq!(stitched.restore_properties.len(), 4);
        assert_eq!(&stitched.restore_properties[0].fourcc, b"BNCN");

        let parsed = parse_img4_envelope(&stitched.img4_bytes).unwrap();
        assert_eq!(&parsed.fourcc, b"rkrn");
        assert_eq!(parsed.payload, b"KERNEL_PAYLOAD_BYTES");
        assert_eq!(parsed.im4m_ticket, b"FAKE_AP_IMG4_TICKET");
        assert!(parsed.im4r_raw.is_some());
    }
}
