//! Price parsing: `79.99 USD`, major units, uppercase ISO 4217 code.
//! Decimal arithmetic only, never floats.

use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Price {
    pub amount: Decimal,
    pub currency: String,
}

impl Price {
    /// The amount in integer minor units, when it converts exactly.
    pub fn to_minor(&self) -> Option<i64> {
        let exp = minor_units(&self.currency)?;
        let factor = Decimal::from(10i64.checked_pow(exp)?);
        let minor = self.amount.checked_mul(factor)?;
        if minor.fract().is_zero() {
            minor.trunc().to_i64()
        } else {
            None
        }
    }

    /// Canonical text, used for comparisons and fingerprints.
    pub fn canonical(&self) -> String {
        format!("{} {}", self.amount.normalize(), self.currency)
    }
}

/// Parse a price as OpenAI and Google write it: `<amount> <CODE>`.
/// The error is a short reason that completes "79,99 EUR ...".
pub fn parse_price(raw: &str) -> Result<Price, String> {
    let v = raw.trim();
    if v.is_empty() {
        return Err("is empty".to_string());
    }
    let mut parts = v.split_whitespace();
    let amount_text = parts.next().unwrap_or("");
    let code = match parts.next() {
        Some(c) => c,
        None => return Err("has no currency code; write it as 79.99 USD".to_string()),
    };
    if parts.next().is_some() {
        return Err("has extra text; write it as 79.99 USD".to_string());
    }
    if code.chars().all(|c| c.is_ascii_digit() || c == '.')
        && amount_text.chars().all(|c| c.is_ascii_alphabetic())
    {
        return Err("puts the currency first; write it as 79.99 USD".to_string());
    }
    let amount = parse_amount(amount_text)?;
    check_currency(code)?;
    Ok(Price {
        amount,
        currency: code.to_string(),
    })
}

/// A plain decimal amount: digits, at most one dot, no thousands separators.
pub fn parse_amount(text: &str) -> Result<Decimal, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("has no amount".to_string());
    }
    if t.starts_with('-') {
        return Err("is negative".to_string());
    }
    if t.contains(',') {
        return Err(
            "uses a comma; write the amount with a dot and no thousands separators".to_string(),
        );
    }
    let ok = t.chars().all(|c| c.is_ascii_digit() || c == '.')
        && t.chars().filter(|c| *c == '.').count() <= 1
        && t.chars().any(|c| c.is_ascii_digit());
    if !ok {
        return Err("has an amount that is not a number".to_string());
    }
    Decimal::from_str(t).map_err(|_| "has an amount that is not a number".to_string())
}

/// Check an ISO 4217 code. The error completes "USD ..." style sentences.
pub fn check_currency(code: &str) -> Result<(), String> {
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!(
            "has currency {code}, which is not a three-letter ISO 4217 code"
        ));
    }
    let upper = code.to_ascii_uppercase();
    if upper != code {
        return Err(format!(
            "has currency {code}; ISO 4217 codes are uppercase ({upper})"
        ));
    }
    if minor_units(code).is_none() {
        return Err(format!(
            "has currency {code}, which is not an ISO 4217 code"
        ));
    }
    Ok(())
}

/// Minor-unit exponent for an ISO 4217 code, or None if unknown.
pub fn minor_units(code: &str) -> Option<u32> {
    // Built once: this runs for every price of every row.
    static CODES: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    let codes = CODES.get_or_init(|| ISO_4217.split_whitespace().collect());
    if !codes.contains(code) {
        return None;
    }
    Some(match code {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        "CLF" | "UYW" => 4,
        _ => 2,
    })
}

/// Active ISO 4217 currency codes, plus a few recently withdrawn ones that
/// feeds may still carry (ANG, BGN, HRK); accepting them is the lenient choice.
const ISO_4217: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB \
BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP \
DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HRK HTG HUF IDR ILS INR IQD \
IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD \
MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN \
PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL THB TJS TMT \
TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAF XCD XCG XOF XPF \
YER ZAR ZMW ZWG";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_units() {
        let p = parse_price("79.99 USD").expect("valid");
        assert_eq!(p.amount, Decimal::new(7999, 2));
        assert_eq!(p.currency, "USD");
        assert_eq!(p.to_minor(), Some(7999));
        assert_eq!(
            parse_price(" 15.00 USD ").map(|p| p.canonical()),
            Ok("15 USD".to_string())
        );
    }

    #[test]
    fn rejects_bad_prices() {
        assert!(parse_price("79.99").is_err());
        assert!(parse_price("79.99 usd").is_err());
        assert!(parse_price("USD 79.99").is_err());
        assert!(parse_price("1,299.00 USD").is_err());
        assert!(parse_price("-1 USD").is_err());
        assert!(parse_price("12.3.4 USD").is_err());
        assert!(parse_price("10 XYZ").is_err());
    }

    #[test]
    fn minor_units_follow_iso() {
        assert_eq!(
            parse_price("1000 JPY").ok().and_then(|p| p.to_minor()),
            Some(1000)
        );
        assert_eq!(parse_price("1.5 JPY").ok().and_then(|p| p.to_minor()), None);
        assert_eq!(
            parse_price("1.234 KWD").ok().and_then(|p| p.to_minor()),
            Some(1234)
        );
    }
}
