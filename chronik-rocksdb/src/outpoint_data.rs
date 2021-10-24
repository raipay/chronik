use std::cmp::Ordering;

use byteorder::BE;
use zerocopy::{AsBytes, FromBytes, Unaligned, U32};

use crate::TxNum;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutpointEntry {
    pub tx_num: u64,
    pub out_idx: u32,
}

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
#[repr(C)]
pub struct OutpointData {
    pub tx_num: TxNum,
    pub out_idx: U32<BE>,
}

impl Ord for OutpointData {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.tx_num.0.get().cmp(&other.tx_num.0.get()) {
            Ordering::Equal => self.out_idx.get().cmp(&other.out_idx.get()),
            ordering => ordering,
        }
    }
}

impl PartialOrd for OutpointData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
