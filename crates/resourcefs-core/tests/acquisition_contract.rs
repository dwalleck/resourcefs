use std::time::Duration;

use resourcefs_core::{
    AcquisitionLimitKind, ErrorReason, HttpStatus, LimitDetail, ReadAcquisitionLimits,
};

#[test]
fn rejects_zero_and_above_hard_in_every_dimension() {
    let caps = [10, 8_388_608, 16_777_216, 16_777_216, 4_194_304];
    for (dimension, cap) in caps.into_iter().enumerate() {
        for value in [0, cap + 1] {
            let mut values = [None; 5];
            values[dimension] = Some(value);
            let error = ReadAcquisitionLimits::new(
                values[0], None, values[1], values[2], values[3], values[4],
            )
            .expect_err("invalid dimension must not be clamped or defaulted");
            assert_eq!(
                error.details().expect("typed refusal").reason(),
                ErrorReason::InvalidAcquisitionLimit
            );
            if dimension == 4 {
                assert_eq!(
                    error
                        .details()
                        .expect("typed refusal")
                        .limit()
                        .expect("limit detail")
                        .kind(),
                    AcquisitionLimitKind::DecodedContentBytes
                );
            }
        }
    }
    for timeout in [
        Duration::ZERO,
        Duration::from_secs(30) + Duration::from_nanos(1),
        Duration::MAX,
    ] {
        let error = ReadAcquisitionLimits::new(None, Some(timeout), None, None, None, None)
            .expect_err("invalid duration must fail");
        assert_eq!(
            error.details().expect("typed refusal").reason(),
            ErrorReason::InvalidAcquisitionLimit
        );
    }
    let hard = ReadAcquisitionLimits::new(
        Some(10),
        Some(Duration::from_secs(30)),
        Some(8_388_608),
        Some(16_777_216),
        Some(16_777_216),
        Some(4_194_304),
    )
    .expect("inclusive hard boundary");
    let minimum = ReadAcquisitionLimits::new(
        Some(1),
        Some(Duration::from_nanos(1)),
        Some(1),
        Some(1),
        Some(1),
        Some(1),
    )
    .expect("positive boundary");
    assert_eq!(hard.intersect(minimum), minimum);
}

#[test]
fn intersection_lowers_unequal_dimensions_independently() {
    let left = ReadAcquisitionLimits::new(
        Some(2),
        Some(Duration::from_secs(20)),
        Some(30),
        Some(400),
        Some(50),
        Some(70),
    )
    .expect("left policy");
    let right = ReadAcquisitionLimits::new(
        Some(8),
        Some(Duration::from_secs(3)),
        Some(300),
        Some(40),
        Some(500),
        Some(60),
    )
    .expect("right policy");
    let expected = ReadAcquisitionLimits::new(
        Some(2),
        Some(Duration::from_secs(3)),
        Some(30),
        Some(40),
        Some(50),
        Some(60),
    )
    .expect("independent minima");
    assert_eq!(left.intersect(right), expected);
    assert_eq!(right.intersect(left), expected);
    assert_eq!(
        expected.intersect(ReadAcquisitionLimits::default()),
        expected
    );
}

#[test]
fn decoded_content_bound_is_stable_and_inclusive() {
    let exact = ReadAcquisitionLimits::new(None, None, None, None, None, Some(4_194_304))
        .expect("hard decoded boundary");
    assert_eq!(exact.max_decoded_bytes(), 4_194_304);
    assert_eq!(
        AcquisitionLimitKind::DecodedContentBytes.as_str(),
        "decoded_content_bytes"
    );
    let zero = ReadAcquisitionLimits::new(None, None, None, None, None, Some(0))
        .expect_err("zero decoded content is invalid");
    assert_eq!(
        zero.details()
            .expect("typed refusal")
            .limit()
            .expect("decoded limit")
            .kind(),
        AcquisitionLimitKind::DecodedContentBytes
    );
    let above = ReadAcquisitionLimits::new(None, None, None, None, None, Some(4_194_305))
        .expect_err("above-hard decoded content is invalid");
    assert_eq!(
        above
            .details()
            .expect("typed refusal")
            .limit()
            .expect("decoded limit")
            .kind(),
        AcquisitionLimitKind::DecodedContentBytes
    );
}

#[test]
fn bounded_details_reject_invalid_status_and_zero_limit() {
    for status in [0, 99, 1_000, u16::MAX] {
        assert!(HttpStatus::new(status).is_err());
    }
    for status in [100, 999] {
        assert_eq!(
            HttpStatus::new(status).expect("three-digit status").get(),
            status
        );
    }
    assert!(LimitDetail::new(AcquisitionLimitKind::Attempts, 0, Some(1)).is_err());
    // Zero observation is valid: no work may have completed when a positive bound fails.
    assert!(LimitDetail::new(AcquisitionLimitKind::Attempts, 1, Some(0)).is_ok());
}
