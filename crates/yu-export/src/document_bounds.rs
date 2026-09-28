//! Unit guard checks use the real limits but do not allocate a 256 MiB output.
use super::*;
#[test]
fn output_and_resource_byte_guards_accept_limit_reject_next_and_overflow() {
    for limit in [MAX_RESOURCE_OUTPUT_BYTES, MAX_OUTPUT_BYTES] {
        assert_eq!(
            checked_length(limit - 1, 1, limit).expect("last byte"),
            limit
        );
        assert_eq!(checked_length(limit, 0, limit).expect("at limit"), limit);
        assert!(checked_length(limit, 1, limit).is_err());
        assert!(checked_length(usize::MAX, 1, limit).is_err());
    }
}
#[test]
fn slot_preflight_includes_repeated_values_and_literal_tail() {
    let mut slots = Slots::new("");
    let token = slots.insert("x".repeat(1024 * 1024), false);
    assert!(slots.expand(&token.repeat(257)).is_err());
    assert!(slots.expand(&(token.repeat(256) + "x")).is_err());
    let mut small = Slots::new("");
    let token = small.insert("中文🙂".into(), false);
    assert_eq!(
        small
            .expand(&format!("start {token} end"))
            .expect("expansion"),
        "start 中文🙂 end"
    );
    assert!(small.expand("YUEXPORTSLOT999Z").is_err());
}
#[test]
fn final_join_preflights_all_parts_before_allocation() {
    let chunk = "x".repeat(1024 * 1024);
    assert!(join_html(&vec![chunk.as_str(); 257]).is_err());
    assert_eq!(
        join_html(&["head", "中文", "tail"]).expect("join"),
        "head中文tail"
    );
}
