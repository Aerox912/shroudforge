use super::extract_reflection_data;
fn u64at(b: &mut [u8], p: usize, n: u64) {
    b[p..p + 8].copy_from_slice(&n.to_le_bytes())
}
fn u32at(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes())
}
fn ptr(b: &mut [u8], p: usize, to: usize) {
    u64at(b, p, 0x140000000 + to as u64)
}
pub(crate) fn fixture() -> Vec<u8> {
    let mut b = vec![0u8; 4096];
    b[..2].copy_from_slice(b"MZ");
    u32at(&mut b, 60, 128);
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[134..136].copy_from_slice(&1u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    u64at(&mut b, 176, 0x140000000);
    b[392..398].copy_from_slice(b".rdata");
    u32at(&mut b, 404, 512);
    u32at(&mut b, 408, 3584);
    u32at(&mut b, 412, 512);
    b[512..522].copy_from_slice(b"BlobString");
    b[544..550].copy_from_slice(b"uint32");
    ptr(&mut b, 600, 512);
    u64at(&mut b, 608, 10);
    ptr(&mut b, 616, 512);
    u64at(&mut b, 624, 10);
    ptr(&mut b, 632, 512);
    u64at(&mut b, 640, 10);
    ptr(&mut b, 648, 1000);
    u32at(&mut b, 664, 8);
    b[668..670].copy_from_slice(&8u16.to_le_bytes());
    b[670..672].copy_from_slice(&8u16.to_le_bytes());
    b[676] = 25;
    u32at(&mut b, 680, kfc::hash::fnv("BlobString"));
    ptr(&mut b, 800, 600);
    ptr(&mut b, 808, 1800);
    ptr(&mut b, 840, 800);
    u64at(&mut b, 848, 2);
    ptr(&mut b, 1000, 1100);
    u64at(&mut b, 1008, 4);
    b[1100..1104].copy_from_slice(b"keen");
    for at in [1800, 1816, 1832] {
        ptr(&mut b, at, 544);
        u64at(&mut b, at + 8, 6);
    }
    ptr(&mut b, 1848, 1000);
    u32at(&mut b, 1864, 4);
    b[1868..1870].copy_from_slice(&4u16.to_le_bytes());
    b[1870..1872].copy_from_slice(&4u16.to_le_bytes());
    b[1876] = 6;
    u32at(&mut b, 1880, kfc::hash::fnv("uint32"));
    b
}
#[test]
fn strict_static_valid_zero_optional_preserves_discovery_and_schema() {
    let t = extract_reflection_data(&fixture()).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].name, "BlobString");
    assert!(t[0].inner_type.is_none());
    assert!(t[0].attributes.is_empty());
}
#[test]
fn strict_static_duplicate_field_names_must_not_be_silently_overwritten() {
    let mut b = fixture();
    b[676] = 18;
    u32at(&mut b, 672, 2);
    ptr(&mut b, 688, 1200);
    b[1400] = b'x';
    for at in [1200, 1248] {
        ptr(&mut b, at, 1400);
        u64at(&mut b, at + 8, 1);
        ptr(&mut b, at + 16, 600);
    }
    assert!(
        extract_reflection_data(&b).is_err(),
        "rawduplicate field metadata was erased"
    );
}
#[test]
fn strict_static_namespace_cycle_must_be_bounded() {
    let mut b = fixture();
    ptr(&mut b, 1016, 1000);
    assert!(extract_reflection_data(&b).is_err());
}

#[test]
fn strict_optional_zero_unknown_and_required_reference_distinctions() {
    let mut b = fixture();
    assert!(extract_reflection_data(&b).is_ok());
    assert_eq!(super::unresolved_optional_count(), 0);
    ptr(&mut b, 656, 3000);
    let t = extract_reflection_data(&b).unwrap();
    assert!(t[0].inner_type.is_none());
    assert_eq!(super::unresolved_optional_count(), 1);
    b[676] = 18;
    u32at(&mut b, 672, 1);
    ptr(&mut b, 688, 1200);
    ptr(&mut b, 1200, 1400);
    b[1400] = b'x';
    u64at(&mut b, 1208, 1);
    ptr(&mut b, 1216, 3000);
    assert!(extract_reflection_data(&b).is_err());
}
#[test]
fn strict_enum_duplicate_values_allowed_but_duplicate_names_refuse() {
    let mut b = fixture();
    b[676] = 12;
    ptr(&mut b, 656, 1800);
    u32at(&mut b, 672, 2);
    ptr(&mut b, 696, 1200);
    b[1400] = b'a';
    b[1401] = b'b';
    for (at, name) in [(1200, 1400), (1240, 1401)] {
        ptr(&mut b, at, name);
        u64at(&mut b, at + 8, 1);
        u64at(&mut b, at + 16, 7);
    }
    let t = extract_reflection_data(&b).unwrap();
    assert_eq!(t[0].enum_fields.len(), 2);
    ptr(&mut b, 1240, 1400);
    assert!(extract_reflection_data(&b).is_err());
}
#[test]
fn strict_attribute_short_name_collision_preserves_both_optional_records() {
    let mut b = fixture();
    ptr(&mut b, 728, 1200);
    u64at(&mut b, 736, 2);
    b[1400] = b'a';
    b[1500] = b'x';
    b[1501] = b'y';
    for (at, meta, value) in [(1200, 1300, 1500), (1224, 1350, 1501)] {
        ptr(&mut b, at, meta);
        ptr(&mut b, at + 8, value);
        u64at(&mut b, at + 16, 1);
        ptr(&mut b, meta, 1000);
        ptr(&mut b, meta + 8, 1400);
        u64at(&mut b, meta + 16, 1);
    }
    let t = extract_reflection_data(&b).unwrap();
    assert_eq!(t[0].attributes.len(), 1);
    let a = super::attribute_collisions();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["previous"]["value"], "x");
    assert_eq!(a[0]["current"]["value"], "y");
    assert!(t[0].attributes["a"].r#type.is_none());
}
#[test]
fn strict_preallocation_ranges_and_pattern_failure_are_bounded() {
    for at in [608, 848] {
        let mut b = fixture();
        u64at(&mut b, at, u64::MAX);
        assert!(extract_reflection_data(&b).is_err());
    }
    let mut b = fixture();
    u64at(&mut b, 712, 0x140000000 + 3000);
    u64at(&mut b, 720, 1048577);
    assert!(extract_reflection_data(&b).is_err());
    let mut b = fixture();
    b[132] = 0;
    assert!(extract_reflection_data(&b).is_err());
    assert!(extract_reflection_data(&[0; 4]).is_err());
}
