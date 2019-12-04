use byteorder::{ByteOrder, ReadBytesExt, WriteBytesExt};
use lazy_static::lazy_static;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Error, ErrorKind, Read, Write};
use std::path::Path;
use std::string::ToString;
use std::sync::Mutex;

pub use compile_time_crc32;

#[macro_export]
macro_rules! hash40 {
    ($lit:literal) => {
        Hash40(($crate::compile_time_crc32::crc32!($lit) as u64) | ($lit.len() as u64) << 32)
    };
}

lazy_static! {
    static ref LABELS: Mutex<HashMap<Hash40, String>> = {
        let l = HashMap::new();
        //TODO: populate the dictionary at compile time with all documented labels
        Mutex::new(l)
    };
}

// value list, automatically converted to hash40's ; read line-by-line
pub fn load_labels<P: AsRef<Path>>(file: P) -> Result<(), Error> {
    match LABELS.lock() {
        Ok(ref mut map) => {
            for l in BufReader::new(File::open(file)?).lines() {
                match l {
                    Ok(line) => {
                        map.insert(to_hash40(&line), line);
                    }
                    Err(_) => continue,
                }
            }
            Ok(())
        }
        // TODO: returning a io:Error here is bad
        Err(_) => Err(Error::new(
            ErrorKind::Other,
            "Failed to access global: LABELS",
        )),
    }
}

// comma-separated hash40/value list ; read line-by-line
pub fn load_custom_labels<P: AsRef<Path>>(file: P) -> Result<(), Error> {
    match LABELS.lock() {
        Ok(ref mut map) => {
            for l in BufReader::new(File::open(file)?).lines() {
                match l {
                    Ok(line) => {
                        let split: Vec<&str> = line.split(',').collect();
                        if split.len() < 2 {
                            continue;
                        }
                        let hash = if split[0].starts_with("0x") {
                            match Hash40::from_hex_str(split[0]) {
                                Ok(h) => h,
                                Err(_) => continue,
                            }
                        } else {
                            continue;
                        };
                        map.insert(hash, String::from(split[1]));
                    }
                    Err(_) => continue,
                }
            }
            Ok(())
        }
        // TODO: returning a io:Error here is bad
        Err(_) => Err(Error::new(
            ErrorKind::Other,
            "Failed to access global: LABELS",
        )),
    }
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hash40(pub u64);

impl Hash40 {
    #[inline]
    pub fn crc(self) -> u32 {
        self.0 as u32
    }

    #[inline]
    pub fn strlen(self) -> u8 {
        (self.0 >> 32) as u8
    }

    pub fn to_label(self) -> String {
        match LABELS.lock() {
            Ok(x) => match x.get(&self) {
                Some(l) => String::from(l),
                None => self.to_string(),
            },
            Err(_) => self.to_string(),
        }
    }

    pub fn from_hex_str(value: &str) -> Result<Self, std::num::ParseIntError> {
        Ok(Hash40(u64::from_str_radix(&value[2..], 16)?))
    }
}

// Hash40 -> string
impl ToString for Hash40 {
    fn to_string(&self) -> String {
        format!("0x{:010x}", self.0)
    }
}

// extension of io::Read capabilities to get Hash40 from stream
pub trait ReadHash40: ReadBytesExt {
    fn read_hash40<T: ByteOrder>(&mut self) -> Result<Hash40, Error>;

    fn read_hash40_with_meta<T: ByteOrder>(&mut self) -> Result<(Hash40, u32), Error>;
}
impl<R: Read> ReadHash40 for R {
    fn read_hash40<T: ByteOrder>(&mut self) -> Result<Hash40, Error> {
        Ok(Hash40(self.read_u64::<T>()? & 0xff_ffff_ffff))
    }

    fn read_hash40_with_meta<T: ByteOrder>(&mut self) -> Result<(Hash40, u32), Error> {
        let long = self.read_u64::<T>()?;
        Ok((Hash40(long & 0xff_ffff_ffff), (long >> 40) as u32))
    }
}

// extension of io::Write capabilities to write Hash40 to stream
pub trait WriteHash40: WriteBytesExt {
    fn write_hash40<T: ByteOrder>(&mut self, hash: Hash40) -> Result<(), Error>;

    fn write_hash40_with_meta<T: ByteOrder>(
        &mut self,
        hash: Hash40,
        meta: u32,
    ) -> Result<(), Error>;
}
impl<W: Write> WriteHash40 for W {
    fn write_hash40<T: ByteOrder>(&mut self, hash: Hash40) -> Result<(), Error> {
        self.write_u64::<T>(hash.0)
    }

    fn write_hash40_with_meta<T: ByteOrder>(
        &mut self,
        hash: Hash40,
        meta: u32,
    ) -> Result<(), Error> {
        self.write_u64::<T>(hash.0 | (meta as u64) << 40)
    }
}

struct Hash40Visitor;

impl<'de> de::Visitor<'de> for Hash40Visitor {
    type Value = Hash40;

    fn expecting(
        &self,
        formatter: &mut std::fmt::Formatter,
    ) -> std::result::Result<(), std::fmt::Error> {
        formatter.write_str(
            "A hex-formatted integer hash value, or a string standing for its reversed form",
        )
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        if value.starts_with("0x") {
            Hash40::from_hex_str(value).map_err(|e| { E::custom(e) })
        } else {
            Ok(to_hash40(value))
        }
    }
}

impl Serialize for Hash40 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_label())
    }
}

impl<'de> Deserialize<'de> for Hash40 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(Hash40Visitor)
    }
}

//exposed function to compute a hash
pub fn to_hash40(word: &str) -> Hash40 {
    Hash40(crc32_with_len(word))
}

fn crc32_with_len(word: &str) -> u64 {
    let mut hash = !0u32;
    let mut len: u8 = 0;
    for b in word.bytes() {
        let shift = hash >> 8;
        let index = (hash ^ (b as u32)) & 0xff;
        hash = shift ^ _CRC_TABLE[index as usize];
        len += 1;
    }
    ((len as u64) << 32) | (!hash as u64)
}

const _CRC_TABLE: [u32; 256] = [
    0x00000000, 0x77073096, 0xee0e612c, 0x990951ba, 0x076dc419, 0x706af48f, 0xe963a535, 0x9e6495a3,
    0x0edb8832, 0x79dcb8a4, 0xe0d5e91e, 0x97d2d988, 0x09b64c2b, 0x7eb17cbd, 0xe7b82d07, 0x90bf1d91,
    0x1db71064, 0x6ab020f2, 0xf3b97148, 0x84be41de, 0x1adad47d, 0x6ddde4eb, 0xf4d4b551, 0x83d385c7,
    0x136c9856, 0x646ba8c0, 0xfd62f97a, 0x8a65c9ec, 0x14015c4f, 0x63066cd9, 0xfa0f3d63, 0x8d080df5,
    0x3b6e20c8, 0x4c69105e, 0xd56041e4, 0xa2677172, 0x3c03e4d1, 0x4b04d447, 0xd20d85fd, 0xa50ab56b,
    0x35b5a8fa, 0x42b2986c, 0xdbbbc9d6, 0xacbcf940, 0x32d86ce3, 0x45df5c75, 0xdcd60dcf, 0xabd13d59,
    0x26d930ac, 0x51de003a, 0xc8d75180, 0xbfd06116, 0x21b4f4b5, 0x56b3c423, 0xcfba9599, 0xb8bda50f,
    0x2802b89e, 0x5f058808, 0xc60cd9b2, 0xb10be924, 0x2f6f7c87, 0x58684c11, 0xc1611dab, 0xb6662d3d,
    0x76dc4190, 0x01db7106, 0x98d220bc, 0xefd5102a, 0x71b18589, 0x06b6b51f, 0x9fbfe4a5, 0xe8b8d433,
    0x7807c9a2, 0x0f00f934, 0x9609a88e, 0xe10e9818, 0x7f6a0dbb, 0x086d3d2d, 0x91646c97, 0xe6635c01,
    0x6b6b51f4, 0x1c6c6162, 0x856530d8, 0xf262004e, 0x6c0695ed, 0x1b01a57b, 0x8208f4c1, 0xf50fc457,
    0x65b0d9c6, 0x12b7e950, 0x8bbeb8ea, 0xfcb9887c, 0x62dd1ddf, 0x15da2d49, 0x8cd37cf3, 0xfbd44c65,
    0x4db26158, 0x3ab551ce, 0xa3bc0074, 0xd4bb30e2, 0x4adfa541, 0x3dd895d7, 0xa4d1c46d, 0xd3d6f4fb,
    0x4369e96a, 0x346ed9fc, 0xad678846, 0xda60b8d0, 0x44042d73, 0x33031de5, 0xaa0a4c5f, 0xdd0d7cc9,
    0x5005713c, 0x270241aa, 0xbe0b1010, 0xc90c2086, 0x5768b525, 0x206f85b3, 0xb966d409, 0xce61e49f,
    0x5edef90e, 0x29d9c998, 0xb0d09822, 0xc7d7a8b4, 0x59b33d17, 0x2eb40d81, 0xb7bd5c3b, 0xc0ba6cad,
    0xedb88320, 0x9abfb3b6, 0x03b6e20c, 0x74b1d29a, 0xead54739, 0x9dd277af, 0x04db2615, 0x73dc1683,
    0xe3630b12, 0x94643b84, 0x0d6d6a3e, 0x7a6a5aa8, 0xe40ecf0b, 0x9309ff9d, 0x0a00ae27, 0x7d079eb1,
    0xf00f9344, 0x8708a3d2, 0x1e01f268, 0x6906c2fe, 0xf762575d, 0x806567cb, 0x196c3671, 0x6e6b06e7,
    0xfed41b76, 0x89d32be0, 0x10da7a5a, 0x67dd4acc, 0xf9b9df6f, 0x8ebeeff9, 0x17b7be43, 0x60b08ed5,
    0xd6d6a3e8, 0xa1d1937e, 0x38d8c2c4, 0x4fdff252, 0xd1bb67f1, 0xa6bc5767, 0x3fb506dd, 0x48b2364b,
    0xd80d2bda, 0xaf0a1b4c, 0x36034af6, 0x41047a60, 0xdf60efc3, 0xa867df55, 0x316e8eef, 0x4669be79,
    0xcb61b38c, 0xbc66831a, 0x256fd2a0, 0x5268e236, 0xcc0c7795, 0xbb0b4703, 0x220216b9, 0x5505262f,
    0xc5ba3bbe, 0xb2bd0b28, 0x2bb45a92, 0x5cb36a04, 0xc2d7ffa7, 0xb5d0cf31, 0x2cd99e8b, 0x5bdeae1d,
    0x9b64c2b0, 0xec63f226, 0x756aa39c, 0x026d930a, 0x9c0906a9, 0xeb0e363f, 0x72076785, 0x05005713,
    0x95bf4a82, 0xe2b87a14, 0x7bb12bae, 0x0cb61b38, 0x92d28e9b, 0xe5d5be0d, 0x7cdcefb7, 0x0bdbdf21,
    0x86d3d2d4, 0xf1d4e242, 0x68ddb3f8, 0x1fda836e, 0x81be16cd, 0xf6b9265b, 0x6fb077e1, 0x18b74777,
    0x88085ae6, 0xff0f6a70, 0x66063bca, 0x11010b5c, 0x8f659eff, 0xf862ae69, 0x616bffd3, 0x166ccf45,
    0xa00ae278, 0xd70dd2ee, 0x4e048354, 0x3903b3c2, 0xa7672661, 0xd06016f7, 0x4969474d, 0x3e6e77db,
    0xaed16a4a, 0xd9d65adc, 0x40df0b66, 0x37d83bf0, 0xa9bcae53, 0xdebb9ec5, 0x47b2cf7f, 0x30b5ffe9,
    0xbdbdf21c, 0xcabac28a, 0x53b39330, 0x24b4a3a6, 0xbad03605, 0xcdd70693, 0x54de5729, 0x23d967bf,
    0xb3667a2e, 0xc4614ab8, 0x5d681b02, 0x2a6f2b94, 0xb40bbe37, 0xc30c8ea1, 0x5a05df1b, 0x2d02ef8d,
];
