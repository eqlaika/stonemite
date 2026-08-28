use eqlog::EqSecond;

pub(crate) fn inclusive_seconds(begin: EqSecond, end: EqSecond) -> u64 {
    end.checked_sub(begin)
        .and_then(|delta| delta.checked_add(1))
        .and_then(|seconds| u64::try_from(seconds).ok())
        .unwrap_or(1)
        .max(1)
}

pub(crate) fn union_range_seconds(
    mut ranges: Vec<(EqSecond, EqSecond)>,
    bridge_seconds: i64,
) -> u64 {
    if ranges.is_empty() {
        return 1;
    }
    ranges.sort_by_key(|range| (range.0, range.1));
    let mut total = 0u64;
    let mut current = ranges[0];
    for next in ranges.into_iter().skip(1) {
        let gap = next.0.checked_sub(current.1).unwrap_or(i64::MAX);
        if next.0 <= current.1 || gap <= bridge_seconds {
            if next.1 > current.1 {
                current.1 = next.1;
            }
        } else {
            total = total.saturating_add(inclusive_seconds(current.0, current.1));
            current = next;
        }
    }
    total
        .saturating_add(inclusive_seconds(current.0, current.1))
        .max(1)
}

pub(crate) fn round_rate(damage: u128, seconds: u64) -> Option<u128> {
    let seconds = u128::from(seconds.max(1));
    let quotient = damage / seconds;
    let remainder = damage % seconds;
    quotient.checked_add(u128::from(remainder >= (seconds + 1) / 2))
}

pub(crate) fn ratio_millionths(value: u128, total: u128) -> u32 {
    if total == 0 {
        return 0;
    }
    let bounded = value.min(total);
    mul_div_bounded(bounded, 1_000_000, total) as u32
}

/// Exact floor(a * multiplier / denominator) for a <= denominator and a small
/// multiplier. Product comparison uses a two-limb 256-bit value, so maximum
/// u128 totals never wrap or enter floating point.
fn mul_div_bounded(a: u128, multiplier: u128, denominator: u128) -> u128 {
    let numerator = wide_mul(a, multiplier);
    let mut low = 0u128;
    let mut high = multiplier;
    while low < high {
        let middle = low + (high - low + 1) / 2;
        if wide_mul(middle, denominator) <= numerator {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

fn wide_mul(left: u128, right: u128) -> (u128, u128) {
    let mask = u128::from(u64::MAX);
    let left_low = left & mask;
    let left_high = left >> 64;
    let right_low = right & mask;
    let right_high = right >> 64;
    let low_low = left_low * right_low;
    let low_high = left_low * right_high;
    let high_low = left_high * right_low;
    let high_high = left_high * right_high;
    let carry = (low_low >> 64) + (low_high & mask) + (high_low & mask);
    let low = (low_low & mask) | ((carry & mask) << 64);
    let high = high_high + (low_high >> 64) + (high_low >> 64) + (carry >> 64);
    (high, low)
}

pub fn format_grouped_ascii(value: u128) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    let first = digits.len() % 3;
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && index >= first && (index - first) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_ranges_bridge_at_six_but_not_seven_seconds() {
        let at = |value| EqSecond::new(value);
        assert_eq!(union_range_seconds(vec![(at(10), at(10))], 6), 1);
        assert_eq!(
            union_range_seconds(vec![(at(10), at(12)), (at(18), at(20))], 6),
            11
        );
        assert_eq!(
            union_range_seconds(vec![(at(10), at(12)), (at(19), at(20))], 6),
            5
        );
    }

    #[test]
    fn rates_round_half_up_without_floating_point() {
        assert_eq!(round_rate(2, 4), Some(1));
        assert_eq!(round_rate(1, 3), Some(0));
        assert_eq!(round_rate(2, 3), Some(1));
        assert_eq!(round_rate(u128::MAX, 1), Some(u128::MAX));
    }

    #[test]
    fn ratios_and_grouping_are_bounded() {
        assert_eq!(ratio_millionths(25, 100), 250_000);
        assert_eq!(ratio_millionths(100, 100), 1_000_000);
        assert_eq!(ratio_millionths(200, 100), 1_000_000);
        assert_eq!(format_grouped_ascii(1_234_567), "1,234,567");
        assert_eq!(format_grouped_ascii(12), "12");
    }
}
