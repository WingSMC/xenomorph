use std::ffi::c_void;

pub mod syntax;

pub const ENTRY_POINT: &[u8] = b"xenomorph_plugin_v1\0";
pub const ABI_MAGIC: u64 = 0x58454e4f504c5547;
pub const ABI_MAJOR: u32 = 1;
pub const ABI_MINOR: u32 = 0;
pub const PROTOCOL_VERSION: u32 = 1;
pub const GENERATOR: u64 = 1;
pub const ANALYZER: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl Bytes {
    pub fn borrowed(bytes: &[u8]) -> Self {
        Self {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Buffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub status: u32,
}

#[repr(C)]
pub struct Header {
    pub magic: u64,
    pub major: u32,
    pub minor: u32,
    pub table_size: u32,
    pub protocol: u32,
    pub capabilities: u64,
}

#[repr(C)]
pub struct Api {
    pub header: Header,
    pub describe: unsafe extern "C" fn() -> Buffer,
    pub initialize: unsafe extern "C" fn(Bytes, *mut *mut c_void) -> Buffer,
    pub run: unsafe extern "C" fn(*mut c_void, Bytes) -> Buffer,
    pub destroy: unsafe extern "C" fn(*mut c_void) -> Buffer,
    pub release: unsafe extern "C" fn(Buffer),
}

impl Header {
    pub const fn new(capabilities: u64) -> Self {
        Self {
            magic: ABI_MAGIC,
            major: ABI_MAJOR,
            minor: ABI_MINOR,
            table_size: std::mem::size_of::<Api>() as u32,
            protocol: PROTOCOL_VERSION,
            capabilities,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.magic != ABI_MAGIC
            || self.major != ABI_MAJOR
            || self.protocol != PROTOCOL_VERSION
            || self.table_size < std::mem::size_of::<Api>() as u32
        {
            return Err(format!(
                "Incompatible plugin ABI: expected ABI {ABI_MAJOR}.{ABI_MINOR}, protocol \
                 {PROTOCOL_VERSION}, table >= {}; received ABI {}.{}, protocol {}, table {}",
                std::mem::size_of::<Api>(),
                self.major,
                self.minor,
                self.protocol,
                self.table_size,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_header_before_accessing_callbacks() {
        let mut header = Header::new(GENERATOR);
        assert!(header.validate().is_ok());
        header.table_size -= 1;
        assert!(header.validate().is_err());
        header = Header::new(ANALYZER);
        header.major += 1;
        assert!(header.validate().is_err());
    }

    #[test]
    fn header_is_at_start_of_table() {
        assert_eq!(std::mem::offset_of!(Api, header), 0);
    }
}
