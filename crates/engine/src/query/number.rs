use std::cmp::Ordering;

use super::Error;

// sign * digits * 10^power; no expanded exponent or binary floating point.
pub(crate) struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    power: i32,
}

impl Decimal {
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > 256 {
            return Err(Error::UnsupportedNumber);
        }
        // Also protect this private helper against malformed standalone input.
        text.parse::<serde_json::Number>()
            .map_err(|_| Error::InvalidQuery)?;
        let (mantissa, exponent) = match text.find(['e', 'E']) {
            Some(i) => (
                &text[..i],
                text[i + 1..]
                    .parse::<i32>()
                    .map_err(|_| Error::UnsupportedNumber)?,
            ),
            None => (text, 0),
        };
        if !(-10000..=10000).contains(&exponent) {
            return Err(Error::UnsupportedNumber);
        }
        let negative = mantissa.starts_with('-');
        let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
        let fraction = mantissa.find('.').map_or(0, |i| mantissa.len() - i - 1);
        let mut digits: Vec<_> = mantissa
            .bytes()
            .filter(|b| *b != b'.')
            .skip_while(|b| *b == b'0')
            .collect();
        if digits.is_empty() {
            return Ok(Self {
                negative: false,
                digits: vec![b'0'],
                power: 0,
            });
        }
        let mut power = exponent - fraction as i32;
        while digits.last() == Some(&b'0') {
            digits.pop();
            power += 1;
        }
        Ok(Self {
            negative,
            digits,
            power,
        })
    }

    pub(crate) fn compare(&self, other: &Self) -> Ordering {
        let is_zero = |number: &Self| number.digits.as_slice() == b"0";
        if is_zero(self) || is_zero(other) {
            match (is_zero(self), is_zero(other)) {
                (true, true) => Ordering::Equal,
                (true, false) => {
                    if other.negative {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (false, true) => {
                    if self.negative {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (false, false) => Ordering::Equal,
            }
        } else if self.negative != other.negative {
            if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            let order = (self.digits.len() as i32 + self.power)
                .cmp(&(other.digits.len() as i32 + other.power))
                .then_with(|| {
                    let width = self.digits.len().max(other.digits.len());
                    self.digits
                        .iter()
                        .copied()
                        .chain(std::iter::repeat(b'0'))
                        .take(width)
                        .cmp(
                            other
                                .digits
                                .iter()
                                .copied()
                                .chain(std::iter::repeat(b'0'))
                                .take(width),
                        )
                });
            if self.negative {
                order.reverse()
            } else {
                order
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_comparison() {
        for (a, b, expected) in [
            ("1", "1.0", Ordering::Equal),
            ("1.00", "1e0", Ordering::Equal),
            ("-0", "0e10000", Ordering::Equal),
            ("0", "0.001", Ordering::Less),
            ("0", "-0.001", Ordering::Greater),
            ("-2", "-10", Ordering::Greater),
            ("9007199254740992", "9007199254740993", Ordering::Less),
            ("0.1000000000000000001", "0.1", Ordering::Greater),
            ("1e10000", "9e9999", Ordering::Greater),
            ("1e-10000", "1e-9999", Ordering::Less),
            ("12.30", "12.3001", Ordering::Less),
        ] {
            let a = Decimal::parse(a).unwrap();
            let b = Decimal::parse(b).unwrap();
            assert_eq!(a.compare(&b), expected);
            assert_eq!(b.compare(&a), expected.reverse());
        }
    }

    #[test]
    fn number_limits_and_invalid_tokens_are_explicit() {
        for text in [
            "1e10001",
            "1e-10001",
            "1e999999999999999999",
            &"1".repeat(257),
        ] {
            assert!(matches!(
                Decimal::parse(text),
                Err(Error::UnsupportedNumber)
            ));
        }
        for text in ["NaN", "01", "1.", "+1"] {
            assert!(Decimal::parse(text).is_err());
        }
        assert!(Decimal::parse(&"1".repeat(256)).is_ok());
    }

    #[test]
    fn decimal_order_matches_an_independent_rational_oracle() {
        let samples: Vec<_> = (-9i64..=9)
            .flat_map(|numerator| {
                (0u32..=3).map(move |scale| {
                    (
                        numerator,
                        scale,
                        Decimal::parse(&format!("{numerator}e-{scale}")).unwrap(),
                    )
                })
            })
            .collect();
        for (a, a_scale, left) in &samples {
            for (b, b_scale, right) in &samples {
                let expected = (a * 10i64.pow(*b_scale)).cmp(&(b * 10i64.pow(*a_scale)));
                assert_eq!(left.compare(right), expected);
            }
        }
    }
}
