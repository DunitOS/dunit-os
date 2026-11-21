//! Static x86-64 ELF records and bounded program-header traversal.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ElfHeader {
    pub magic: [u8; 4],
    pub class: u8,
    pub data: u8,
    pub version: u8,
    pub os_abi: u8,
    pub abi_version: u8,
    pub padding: [u8; 7],
    pub elf_type: u16,
    pub machine: u16,
    pub version2: u32,
    pub entry: u64,
    pub phoff: u64,
    pub shoff: u64,
    pub flags: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELF_CLASS_64: u8 = 2;
pub const ELF_DATA_LSB: u8 = 1;
pub const ELF_MACHINE_X86_64: u16 = 0x3E;
pub const ELF_TYPE_EXEC: u16 = 2;
pub const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_STACK: u32 = 0x6474_e551;
pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    InvalidClass,
    InvalidArchitecture,
    InvalidType,
    UnsupportedDynamicLinking,
    InvalidProgramHeader,
    TooManyProgramHeaders,
}

pub struct ElfParser<'a> {
    pub(super) data: &'a [u8],
    pub(super) header: ElfHeader,
}

pub(super) struct AuxProgramHeaders<'a> {
    pub mapped_address: Option<u64>,
    pub bytes: &'a [u8],
}

impl<'a> ElfParser<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, ElfError> {
        if data.len() < core::mem::size_of::<ElfHeader>() {
            return Err(ElfError::InvalidMagic);
        }

        let header = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const ElfHeader) };

        if header.magic != ELF_MAGIC {
            return Err(ElfError::InvalidMagic);
        }

        if header.class != ELF_CLASS_64 {
            return Err(ElfError::InvalidClass);
        }

        if header.machine != ELF_MACHINE_X86_64 {
            return Err(ElfError::InvalidArchitecture);
        }

        if header.elf_type != ELF_TYPE_EXEC {
            return Err(ElfError::InvalidType);
        }

        if header.data != ELF_DATA_LSB
            || header.version != 1
            || header.version2 != 1
            || header.ehsize as usize != core::mem::size_of::<ElfHeader>()
        {
            return Err(ElfError::InvalidProgramHeader);
        }

        let parser = Self { data, header };
        // This loader enters the main image directly. Refuse images that
        // need a linker before mapping any bytes or preparing an entry point.
        if parser
            .program_headers()?
            .any(|ph| matches!(ph.p_type, PT_INTERP | PT_DYNAMIC))
        {
            return Err(ElfError::UnsupportedDynamicLinking);
        }
        Ok(parser)
    }

    pub fn header(&self) -> &ElfHeader {
        &self.header
    }

    pub fn entry_point(&self) -> u64 {
        self.header.entry
    }

    fn program_header_bytes(&self) -> Result<&'a [u8], ElfError> {
        let phoff =
            usize::try_from(self.header.phoff).map_err(|_| ElfError::InvalidProgramHeader)?;
        let phnum = self.header.phnum as usize;
        let phentsize = self.header.phentsize as usize;

        if phentsize != core::mem::size_of::<ProgramHeader>() || phoff % 8 != 0 {
            return Err(ElfError::InvalidProgramHeader);
        }
        let end = phnum
            .checked_mul(phentsize)
            .and_then(|size| phoff.checked_add(size))
            .ok_or(ElfError::InvalidProgramHeader)?;
        let bytes = self
            .data
            .get(phoff..end)
            .ok_or(ElfError::InvalidProgramHeader)?;
        if phnum > 256 {
            return Err(ElfError::TooManyProgramHeaders);
        }
        Ok(bytes)
    }

    pub fn program_headers(&self) -> Result<ProgramHeaderIterator<'a>, ElfError> {
        Ok(ProgramHeaderIterator {
            entries: self
                .program_header_bytes()?
                .chunks_exact(core::mem::size_of::<ProgramHeader>()),
        })
    }

    /// Locate the complete header table in a file-backed LOAD. A declared PHDR
    /// must describe the same mapped bytes. The static stack can copy the table
    /// when the image has no mapped or declared table.
    pub(super) fn aux_program_headers(&self) -> Result<AuxProgramHeaders<'a>, ElfError> {
        let bytes = self.program_header_bytes()?;
        let table_start = self.header.phoff;
        let table_end = table_start
            .checked_add(bytes.len() as u64)
            .ok_or(ElfError::InvalidProgramHeader)?;
        let mut mapped = None;
        let mut declared = None;
        for ph in self.program_headers()? {
            if ph.p_type == PT_PHDR {
                if declared.replace(ph.p_vaddr).is_some()
                    || ph.p_offset != table_start
                    || ph.p_filesz != bytes.len() as u64
                    || ph.p_memsz != ph.p_filesz
                {
                    return Err(ElfError::InvalidProgramHeader);
                }
            }
            if ph.p_type != PT_LOAD || ph.p_offset > table_start {
                continue;
            }
            let file_end = ph
                .p_offset
                .checked_add(ph.p_filesz)
                .ok_or(ElfError::InvalidProgramHeader)?;
            if file_end < table_end {
                continue;
            }
            let address = ph
                .p_vaddr
                .checked_add(table_start - ph.p_offset)
                .ok_or(ElfError::InvalidProgramHeader)?;
            address
                .checked_add(bytes.len() as u64)
                .ok_or(ElfError::InvalidProgramHeader)?;
            if mapped
                .replace(address)
                .is_some_and(|previous| previous != address)
            {
                return Err(ElfError::InvalidProgramHeader);
            }
        }
        if declared.is_some() && declared != mapped {
            return Err(ElfError::InvalidProgramHeader);
        }
        Ok(AuxProgramHeaders {
            mapped_address: mapped,
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(count: u16) -> Vec<u8> {
        let mut bytes = vec![0; 64 + count as usize * 56];
        bytes[..4].copy_from_slice(&ELF_MAGIC);
        bytes[4] = ELF_CLASS_64;
        bytes[5] = ELF_DATA_LSB;
        bytes[6] = 1;
        bytes[16..18].copy_from_slice(&ELF_TYPE_EXEC.to_le_bytes());
        bytes[18..20].copy_from_slice(&ELF_MACHINE_X86_64.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x400000u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&count.to_le_bytes());
        for i in 0..count as usize {
            let offset = 64 + i * 56;
            bytes[offset..offset + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
            bytes[offset + 16..offset + 24]
                .copy_from_slice(&(0x400000 + i as u64 * 4096).to_le_bytes());
        }
        bytes
    }

    fn set_segment(bytes: &mut [u8], slot: usize, kind: u32, offset: u64, address: u64, size: u64) {
        let start = 64 + slot * 56;
        bytes[start..start + 4].copy_from_slice(&kind.to_le_bytes());
        for (field, value) in [(8, offset), (16, address), (32, size), (40, size)] {
            bytes[start + field..start + field + 8].copy_from_slice(&value.to_le_bytes());
        }
    }

    #[test]
    fn dynamic_segments_are_rejected_even_in_et_exec() {
        for kind in [PT_INTERP, PT_DYNAMIC] {
            for slot in [0, 1] {
                let mut bytes = image(2);
                set_segment(&mut bytes, slot, kind, 0, 0, 0);
                assert!(matches!(
                    ElfParser::new(&bytes),
                    Err(ElfError::UnsupportedDynamicLinking)
                ));
            }
        }
    }

    #[test]
    fn aux_headers_locate_file_backing_and_preserve_the_static_stack_fallback() {
        let mut bytes = image(2);
        let headers = ElfParser::new(&bytes)
            .unwrap()
            .aux_program_headers()
            .unwrap();
        assert_eq!(headers.mapped_address, None);
        assert_eq!(headers.bytes, &bytes[64..]);

        let table_size = 2 * 56;
        set_segment(&mut bytes, 0, PT_LOAD, 0, 0x400000, 64 + table_size);
        set_segment(&mut bytes, 1, PT_PHDR, 64, 0x400040, table_size);
        let headers = ElfParser::new(&bytes)
            .unwrap()
            .aux_program_headers()
            .unwrap();
        assert_eq!(headers.mapped_address, Some(0x400040));
        assert_eq!(headers.bytes, &bytes[64..]);

        set_segment(&mut bytes, 1, PT_TLS, 0, 0, 0);
        let headers = ElfParser::new(&bytes)
            .unwrap()
            .aux_program_headers()
            .unwrap();
        assert_eq!(headers.mapped_address, Some(0x400040));
    }

    #[test]
    fn aux_headers_reject_inconsistent_declarations_and_ambiguous_or_wrapping_addresses() {
        let table_size = 2 * 56;
        let total = 64 + table_size;
        for (kind, offset, address, size) in [
            (PT_PHDR, 64, 0x401040, table_size),
            (PT_PHDR, 65, 0x400040, table_size),
            (PT_PHDR, 64, 0x400040, table_size - 1),
            (PT_LOAD, 0, 0x401000, total),
            (PT_LOAD, 0, u64::MAX - 64, total),
        ] {
            let mut bytes = image(2);
            set_segment(&mut bytes, 0, PT_LOAD, 0, 0x400000, total);
            set_segment(&mut bytes, 1, kind, offset, address, size);
            assert!(matches!(
                ElfParser::new(&bytes).unwrap().aux_program_headers(),
                Err(ElfError::InvalidProgramHeader)
            ));
        }
        let mut bytes = image(2);
        set_segment(&mut bytes, 0, PT_LOAD, 0, 0x400000, total - 1);
        set_segment(&mut bytes, 1, PT_PHDR, 64, 0x400040, table_size);
        assert!(matches!(
            ElfParser::new(&bytes).unwrap().aux_program_headers(),
            Err(ElfError::InvalidProgramHeader)
        ));
    }

    #[test]
    fn unaligned_image_preserves_header_and_all_records() {
        for count in [0, 1, 2, 16, 256] {
            for padding in 0..8 {
                let mut bytes = vec![0; padding];
                bytes.extend_from_slice(&image(count));
                let parser = ElfParser::new(&bytes[padding..]).unwrap();
                assert_eq!(parser.header().phnum, count);
                assert_eq!(parser.entry_point(), 0x400000);
                let mut headers = parser.program_headers().unwrap();
                for i in 0..count {
                    let header = headers.next().unwrap();
                    assert_eq!(header.p_type, PT_LOAD);
                    assert_eq!(header.p_vaddr, 0x400000 + i as u64 * 4096);
                }
                assert!(headers.next().is_none());
                assert!(headers.next().is_none());
            }
        }
    }

    #[test]
    fn bad_magic_class_encoding_and_ident_version_are_rejected() {
        let original = image(1);
        for offset in 0..7 {
            for byte in 0..=255 {
                if byte == original[offset] {
                    continue;
                }
                let mut bytes = original.clone();
                bytes[offset] = byte;
                assert!(
                    ElfParser::new(&bytes).is_err(),
                    "offset={offset} value={byte}"
                );
            }
        }
    }

    #[test]
    fn unsupported_machine_type_version_and_header_size_are_rejected() {
        for (offset, supported) in [(16, ELF_TYPE_EXEC), (18, ELF_MACHINE_X86_64), (52, 64)] {
            for value in 0..=u16::MAX {
                if value == supported {
                    continue;
                }
                let mut bytes = image(1);
                bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                assert!(ElfParser::new(&bytes).is_err());
            }
        }
        for version in [0, 2, 256, u32::MAX] {
            let mut bytes = image(1);
            bytes[20..24].copy_from_slice(&version.to_le_bytes());
            assert!(ElfParser::new(&bytes).is_err());
        }
    }

    #[test]
    fn truncated_headers_and_overflowing_tables_are_rejected_before_loading() {
        let original = image(3);
        for length in 0..original.len() {
            assert!(ElfParser::new(&original[..length]).is_err());
        }
        for offset in [1, 63, 65, 128, u64::MAX - 7, u64::MAX] {
            let mut bytes = original.clone();
            bytes[32..40].copy_from_slice(&offset.to_le_bytes());
            assert!(ElfParser::new(&bytes).is_err());
        }
        for size in [0u16, 8, 55, 57, u16::MAX] {
            let mut bytes = original.clone();
            bytes[54..56].copy_from_slice(&size.to_le_bytes());
            assert!(ElfParser::new(&bytes).is_err());
        }
        assert!(matches!(
            ElfParser::new(&image(257)),
            Err(ElfError::TooManyProgramHeaders)
        ));
    }
}

pub struct ProgramHeaderIterator<'a> {
    entries: core::slice::ChunksExact<'a, u8>,
}

impl<'a> Iterator for ProgramHeaderIterator<'a> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.entries.next()?;
        // Each entry has exactly the size of ProgramHeader and may be unaligned.
        Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<ProgramHeader>()) })
    }
}
