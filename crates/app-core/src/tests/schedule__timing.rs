use super::*;

#[test]
fn official_clock_keeps_existing_periods_and_adds_evening_classes() {
    let table = default_teaching_periods();
    assert_eq!(
        clock_for_periods(&table, [1, 2]).unwrap(),
        ("08:50", "10:30")
    );
    assert_eq!(
        clock_for_periods(&table, [5, 8]).unwrap(),
        ("13:30", "17:05")
    );
    assert_eq!(
        clock_for_periods(&table, [11, 12]).unwrap(),
        ("19:05", "20:45")
    );
}

#[test]
fn explicit_table_is_strict_and_atomic() {
    let mut table = default_teaching_periods();
    table[2].start_time = "12:00".into();
    assert!(validate_teaching_periods(&table).is_err());
}
