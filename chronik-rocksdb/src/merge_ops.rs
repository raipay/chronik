use zerocopy::{AsBytes, FromBytes, Unaligned};

use crate::data::{interpret, interpret_slice};

pub const PREFIX_INSERT: u8 = b'I';
pub const PREFIX_DELETE: u8 = b'D';

pub fn partial_merge_ordered_list<T: AsBytes + FromBytes + Unaligned + Clone + Ord>(
    _key: &[u8],
    _existing_value: Option<&[u8]>,
    _operands: &mut rocksdb::MergeOperands,
) -> Option<Vec<u8>> {
    None
}

pub fn full_merge_ordered_list<T: AsBytes + FromBytes + Unaligned + Clone + Ord>(
    _key: &[u8],
    existing_value: Option<&[u8]>,
    operands: &mut rocksdb::MergeOperands,
) -> Option<Vec<u8>> {
    let mut entries = match existing_value {
        Some(existing_entries) => {
            let entries = interpret_slice::<T>(existing_entries).unwrap();
            entries.to_vec()
        }
        None => vec![],
    };
    for operand in operands {
        match *operand.get(0).unwrap() {
            PREFIX_INSERT => {
                let entry = interpret(&operand[1..]).unwrap();
                if let Err(insert_idx) = entries.binary_search(entry) {
                    entries.insert(insert_idx, entry.clone());
                }
            }
            PREFIX_DELETE => {
                let entry = interpret(&operand[1..]).unwrap();
                if let Ok(delete_idx) = entries.binary_search(entry) {
                    entries.remove(delete_idx);
                }
            }
            b => {
                panic!("Wrong merge byte: {}", b);
            },
        }
    }
    Some(entries.as_slice().as_bytes().to_vec())
}
