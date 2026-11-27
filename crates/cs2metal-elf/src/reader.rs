use crate::error::{Error, Result};

/// Bounds-checked little-endian reads over a byte slice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn bytes(&self, offset: u64, len: u64) -> Result<&'a [u8]> {
        let oob = Error::OutOfBounds { offset, len };
        let start = usize::try_from(offset).map_err(|_| oob.clone())?;
        let size = usize::try_from(len).map_err(|_| oob.clone())?;
        let end = start.checked_add(size).ok_or(oob.clone())?;
        self.data.get(start..end).ok_or(oob)
    }

    fn array<const N: usize>(&self, offset: u64) -> Result<[u8; N]> {
        let bytes = self.bytes(offset, N as u64)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    pub(crate) fn u8(&self, offset: u64) -> Result<u8> {
        Ok(self.array::<1>(offset)?[0])
    }

    pub(crate) fn u16(&self, offset: u64) -> Result<u16> {
        self.array(offset).map(u16::from_le_bytes)
    }

    pub(crate) fn u32(&self, offset: u64) -> Result<u32> {
        self.array(offset).map(u32::from_le_bytes)
    }

    pub(crate) fn u64(&self, offset: u64) -> Result<u64> {
        self.array(offset).map(u64::from_le_bytes)
    }

    /// The NUL-terminated string starting at `offset`, decoded lossily.
    pub(crate) fn cstr(&self, offset: u64) -> Result<String> {
        let bad = Error::BadString { offset };
        let start = usize::try_from(offset).map_err(|_| bad.clone())?;
        let tail = self.data.get(start..).ok_or(bad.clone())?;
        let end = tail.iter().position(|&b| b == 0).ok_or(bad)?;
        Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
    }
}

/// `base + index * size`, failing instead of wrapping.
pub(crate) fn offset_of(base: u64, index: u64, size: u64) -> Result<u64> {
    index
        .checked_mul(size)
        .and_then(|delta| base.checked_add(delta))
        .ok_or(Error::OutOfBounds {
            offset: base,
            len: u64::MAX,
        })
}
