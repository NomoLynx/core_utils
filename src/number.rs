use crate::core_utils_error::CoreUtilsError;

pub fn sign<T: PartialOrd + From<i8>>(x: T) -> T {
    if x < T::from(0) {
        T::from(-1)
    } else if x > T::from(0) {
        T::from(1)
    } else {
        T::from(0)
    }
}

pub fn round_up_to_16(x: usize) -> usize {
    (x + 15) & !15
}

pub fn u8_array_to_u32(data: &[u8]) -> Result<u32, &'static str> {
    if data.len() != 4 {
        return Err("Input slice must have exactly 4 elements");
    }

    let result = ((data[0] as u32) << 24)
    | ((data[1] as u32) << 16)
    | ((data[2] as u32) << 8)
    | (data[3] as u32);

    Ok(result)
}

pub fn u8_array_to_u32_little_endian(data: &[u8]) -> Result<u32, &'static str> {
    if data.len() != 4 {
        return Err("Input slice must have exactly 4 elements");
    }

    let result = ((data[0] as u32) << 0)
    | ((data[1] as u32) << 8)
    | ((data[2] as u32) << 16)
    | ((data[3] as u32) << 24);

    Ok(result)
}

/// check if a signed integer can fit in i32
pub fn can_fits_in_32bit(x: i64) -> bool {
    (x >= i32::MIN as i64 && x <= i32::MAX as i64) || 
    (x >= u32::MIN as i64 && x <= u32::MAX as i64)
}

pub fn u32_to_base26(mut number: u32) -> String {
    if number == 0 {
        return "A".to_string();
    }

    let mut result = String::new();

    while number > 0 {
        let remainder = (number % 26) as u8;
        let ch = (b'A' + remainder) as char;
        result.push(ch);
        number /= 26;
    }

    result.chars().rev().collect()
}

macro_rules! parse_unsigned_from_str {
    ($imm:expr, $unsigned:ty, $signed:ty) => {{
        let imm = $imm.replace("_", "");
        if imm.starts_with("0x") {
            let hex_trimmed = imm.trim_start_matches("0x");
            let i = <$unsigned>::from_str_radix(hex_trimmed, 16)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($unsigned))))?;
            Ok(i)
        }
        else if imm.starts_with("0b") {
            let bin_trimmed = imm.trim_start_matches("0b");
            let i = <$unsigned>::from_str_radix(bin_trimmed, 2)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($unsigned))))?;
            Ok(i)
        }
        else if imm.starts_with("0o") {
            let oct_trimmed = imm.trim_start_matches("0o");
            let i = <$unsigned>::from_str_radix(oct_trimmed, 8)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($unsigned))))?;
            Ok(i)
        }
        else if imm.starts_with("-") {
            let i = <$signed>::from_str_radix(&imm, 10)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {} then to {}", stringify!($signed), stringify!($unsigned))))?;
            Ok(i as $unsigned)
        }
        else if imm.starts_with("'") && imm.ends_with("'") && imm.len() == 3 {  // convert char to unsigned
            let chars: Vec<char> = imm.chars().collect();
            Ok(chars[1] as $unsigned)
        }
        else {
            let i = <$unsigned>::from_str_radix(&imm, 10)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($unsigned))))?;
            Ok(i)
        }
    }};
}

macro_rules! parse_signed_from_str {
    ($imm:expr, $signed:ty) => {{
        let imm = $imm.replace("_", "");
        if imm.starts_with("0x") {
            let hex_trimmed = imm.trim_start_matches("0x");
            let i = <$signed>::from_str_radix(hex_trimmed, 16)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($signed))))?;
            Ok(i)
        }
        else if imm.starts_with("0b") {
            let bin_trimmed = imm.trim_start_matches("0b");
            let i = <$signed>::from_str_radix(bin_trimmed, 2)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($signed))))?;
            Ok(i)
        }
        else if imm.starts_with("0o") {
            let oct_trimmed = imm.trim_start_matches("0o");
            let i = <$signed>::from_str_radix(oct_trimmed, 8)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($signed))))?;
            Ok(i)
        }
        else {
            let i = <$signed>::from_str_radix(&imm, 10)
                .map_err(|_| CoreUtilsError::ConversionError(format!("cannot convert imm {imm} to {}", stringify!($signed))))?;
            Ok(i)
        }
    }};
}

pub fn get_u64_from_str(imm:&str) -> Result<u64, CoreUtilsError> {
    parse_unsigned_from_str!(imm, u64, i64)
}

pub fn get_i64_from_str(imm:&str) -> Result<i64, CoreUtilsError> {
    get_u64_from_str(imm)
        .map(|u| u as i64)
}

pub fn get_u32_from_str(imm:&str) -> Result<u32, CoreUtilsError> {
    parse_unsigned_from_str!(imm, u32, i32)
}

pub fn get_i32_from_str(imm:&str) -> Result<i32, CoreUtilsError> {
    parse_signed_from_str!(imm, i32)
}

pub fn get_u16_from_str(imm:&str) -> Result<u16, CoreUtilsError> {
    parse_unsigned_from_str!(imm, u16, i16)
}

pub fn get_u8_from_str(imm:&str) -> Result<u8, CoreUtilsError> {
    parse_unsigned_from_str!(imm, u8, i8)
}