pub mod integer {
    use num_bigint::{BigInt, Sign};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(value: &BigInt, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return serializer.serialize_str(&value.to_string());
        }
        let (sign, bytes) = value.to_bytes_le();
        (
            match sign {
                Sign::Minus => 0u8,
                Sign::NoSign => 1,
                Sign::Plus => 2,
            },
            bytes,
        )
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BigInt, D::Error> {
        if deserializer.is_human_readable() {
            return String::deserialize(deserializer)?
                .parse()
                .map_err(serde::de::Error::custom);
        }
        let (sign, bytes) = <(u8, Vec<u8>)>::deserialize(deserializer)?;
        let sign = match sign {
            0 => Sign::Minus,
            1 => Sign::NoSign,
            2 => Sign::Plus,
            _ => return Err(serde::de::Error::custom("Invalid integer sign")),
        };
        if (sign == Sign::NoSign && bytes.iter().any(|byte| *byte != 0))
            || (sign != Sign::NoSign && bytes.iter().all(|byte| *byte == 0))
        {
            return Err(serde::de::Error::custom("Inconsistent integer sign"));
        }
        Ok(BigInt::from_bytes_le(sign, &bytes))
    }
}

pub mod decimal {
    use bigdecimal::BigDecimal;
    use num_bigint::BigInt;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[derive(Serialize, Deserialize)]
    struct Parts {
        #[serde(with = "super::integer")]
        coefficient: BigInt,
        scale: i64,
    }
    pub fn serialize<S: Serializer>(value: &BigDecimal, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return serializer.serialize_str(&value.to_string());
        }
        let (coefficient, scale) = value.as_bigint_and_exponent();
        Parts { coefficient, scale }.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BigDecimal, D::Error> {
        if deserializer.is_human_readable() {
            return String::deserialize(deserializer)?
                .parse()
                .map_err(serde::de::Error::custom);
        }
        let Parts { coefficient, scale } = Parts::deserialize(deserializer)?;
        Ok(BigDecimal::new(coefficient, scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bigdecimal::BigDecimal;
    use num_bigint::BigInt;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Numbers {
        #[serde(with = "integer")]
        integer: BigInt,
        #[serde(with = "decimal")]
        decimal: BigDecimal,
    }

    #[derive(Deserialize)]
    struct Integer(#[serde(with = "integer")] BigInt);

    #[test]
    fn binary_numbers_preserve_arbitrary_magnitudes_and_decimal_scales() {
        for integer in [
            BigInt::from(0),
            BigInt::from(1),
            -(BigInt::from(1) << 500usize),
        ] {
            for scale in [-1000, 0, 1000] {
                let original = Numbers {
                    integer: integer.clone(),
                    decimal: BigDecimal::new(integer.clone(), scale),
                };
                let bytes = crate::codec::encode(&original).unwrap();
                let decoded: Numbers = crate::codec::decode(&bytes).unwrap();
                assert_eq!(original, decoded);
                assert_eq!(
                    original.decimal.as_bigint_and_exponent(),
                    decoded.decimal.as_bigint_and_exponent()
                );
            }
        }
    }

    #[test]
    fn invalid_signs_are_rejected_before_constructing_numbers() {
        for (sign, magnitude) in [(3u8, vec![1u8]), (1, vec![1]), (0, vec![]), (2, vec![0])] {
            let bytes = crate::codec::encode(&(sign, magnitude)).unwrap();
            assert!(crate::codec::decode::<Integer>(&bytes).is_err());
        }
        let bytes = crate::codec::encode(&(2u8, vec![1u8])).unwrap();
        assert_eq!(
            crate::codec::decode::<Integer>(&bytes).unwrap().0,
            BigInt::from(1)
        );
    }
}
