use bitcoinsuite_error::{ErrorMeta, Result};
use thiserror::Error;
use zerocopy::{FromBytes, LayoutVerified, Unaligned};

#[derive(Debug, Error, ErrorMeta)]
pub enum InterpretError {
    #[critical()]
    #[error("Slice has invalid size: {0}")]
    InvalidSliceSize(usize),
}

use self::InterpretError::*;

pub fn interpret<T: FromBytes + Unaligned>(slice: &[u8]) -> Result<&T> {
    let data = LayoutVerified::<_, T>::new_unaligned(slice).ok_or(InvalidSliceSize(slice.len()))?;
    Ok(data.into_ref())
}

pub fn interpret_slice<T: FromBytes + Unaligned>(slice: &[u8]) -> Result<&[T]> {
    Ok(LayoutVerified::new_slice_unaligned(slice)
        .ok_or(InvalidSliceSize(slice.len()))?
        .into_slice())
}
