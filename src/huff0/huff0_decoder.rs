use crate::decoding::bit_reader_reverse::{BitReaderReversed, GetBitsError};
use crate::fse::{FSEDecoder, FSEDecoderError, FSETable, FSETableError};
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::error::Error as StdError;

pub struct HuffmanTable {
    decode: Vec<Entry>,

    weights: Vec<u8>,
    pub max_num_bits: u8,
    bits: Vec<u8>,
    bit_ranks: Vec<u32>,
    rank_indexes: Vec<usize>,

    fse_table: FSETable,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum HuffmanTableError {
    GetBitsError(GetBitsError),
    FSEDecoderError(FSEDecoderError),
    FSETableError(FSETableError),
    SourceIsEmpty,
    NotEnoughBytesForWeights {
        got_bytes: usize,
        expected_bytes: u8,
    },
    ExtraPadding {
        skipped_bits: i32,
    },
    TooManyWeights {
        got: usize,
    },
    MissingWeights,
    LeftoverIsNotAPowerOf2 {
        got: u32,
    },
    NotEnoughBytesToDecompressWeights {
        have: usize,
        need: usize,
    },
    FSETableUsedTooManyBytes {
        used: usize,
        available_bytes: u8,
    },
    NotEnoughBytesInSource {
        got: usize,
        need: usize,
    },
    WeightBiggerThanMaxNumBits {
        got: u8,
    },
    MaxBitsTooHigh {
        got: u8,
    },
}

#[cfg(feature = "std")]
impl StdError for HuffmanTableError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            HuffmanTableError::GetBitsError(source) => Some(source),
            HuffmanTableError::FSEDecoderError(source) => Some(source),
            HuffmanTableError::FSETableError(source) => Some(source),
            _ => None,
        }
    }
}

impl core::fmt::Display for HuffmanTableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        match self {
            HuffmanTableError::GetBitsError(e) => write!(f, "{:?}", e),
            HuffmanTableError::FSEDecoderError(e) => write!(f, "{:?}", e),
            HuffmanTableError::FSETableError(e) => write!(f, "{:?}", e),
            HuffmanTableError::SourceIsEmpty => write!(f, "Source needs to have at least one byte"),
            HuffmanTableError::NotEnoughBytesForWeights {
                got_bytes,
                expected_bytes,
            } => {
                write!(f, "Header says there should be {} bytes for the weights but there are only {} bytes in the stream",
                    expected_bytes,
                    got_bytes)
            }
            HuffmanTableError::ExtraPadding { skipped_bits } => {
                write!(f,
                    "Padding at the end of the sequence_section was more than a byte long: {} bits. Probably caused by data corruption",
                    skipped_bits,
                )
            }
            HuffmanTableError::TooManyWeights { got } => {
                write!(
                    f,
                    "More than 255 weights decoded (got {} weights). Stream is probably corrupted",
                    got,
                )
            }
            HuffmanTableError::MissingWeights => {
                write!(f, "Can\'t build huffman table without any weights")
            }
            HuffmanTableError::LeftoverIsNotAPowerOf2 { got } => {
                write!(f, "Leftover must be power of two but is: {}", got)
            }
            HuffmanTableError::NotEnoughBytesToDecompressWeights { have, need } => {
                write!(
                    f,
                    "Not enough bytes in stream to decompress weights. Is: {}, Should be: {}",
                    have, need,
                )
            }
            HuffmanTableError::FSETableUsedTooManyBytes {
                used,
                available_bytes,
            } => {
                write!(f,
                    "FSE table used more bytes: {} than were meant to be used for the whole stream of huffman weights ({})",
                    used,
                    available_bytes,
                )
            }
            HuffmanTableError::NotEnoughBytesInSource { got, need } => {
                write!(
                    f,
                    "Source needs to have at least {} bytes, got: {}",
                    need, got,
                )
            }
            HuffmanTableError::WeightBiggerThanMaxNumBits { got } => {
                write!(
                    f,
                    "Cant have weight: {} bigger than max_num_bits: {}",
                    got, MAX_MAX_NUM_BITS,
                )
            }
            HuffmanTableError::MaxBitsTooHigh { got } => {
                write!(
                    f,
                    "max_bits derived from weights is: {} should be lower than: {}",
                    got, MAX_MAX_NUM_BITS,
                )
            }
        }
    }
}

impl From<GetBitsError> for HuffmanTableError {
    fn from(val: GetBitsError) -> Self {
        Self::GetBitsError(val)
    }
}

impl From<FSEDecoderError> for HuffmanTableError {
    fn from(val: FSEDecoderError) -> Self {
        Self::FSEDecoderError(val)
    }
}

impl From<FSETableError> for HuffmanTableError {
    fn from(val: FSETableError) -> Self {
        Self::FSETableError(val)
    }
}

pub struct HuffmanDecoder<'table> {
    table: &'table HuffmanTable,
    pub state: u64,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum HuffmanDecoderError {
    GetBitsError(GetBitsError),
}

impl core::fmt::Display for HuffmanDecoderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HuffmanDecoderError::GetBitsError(e) => write!(f, "{:?}", e),
        }
    }
}

#[cfg(feature = "std")]
impl StdError for HuffmanDecoderError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            HuffmanDecoderError::GetBitsError(source) => Some(source),
        }
    }
}

impl From<GetBitsError> for HuffmanDecoderError {
    fn from(val: GetBitsError) -> Self {
        Self::GetBitsError(val)
    }
}

#[derive(Copy, Clone)]
pub struct Entry {
    symbol: u8,
    num_bits: u8,
}

const MAX_MAX_NUM_BITS: u8 = 11;

fn highest_bit_set(x: u32) -> u32 {
    assert!(x > 0);
    u32::BITS - x.leading_zeros()
}

impl<'t> HuffmanDecoder<'t> {
    pub fn new(table: &'t HuffmanTable) -> HuffmanDecoder<'t> {
        HuffmanDecoder { table, state: 0 }
    }

    pub fn reset(mut self, new_table: Option<&'t HuffmanTable>) {
        self.state = 0;
        if let Some(next_table) = new_table {
            self.table = next_table;
        }
    }

    pub fn decode_symbol(&mut self) -> u8 {
        self.table.decode[self.state as usize].symbol
    }

    pub fn init_state(
        &mut self,
        br: &mut BitReaderReversed<'_>,
    ) -> Result<u8, HuffmanDecoderError> {
        let num_bits = self.table.max_num_bits;
        let new_bits = br.get_bits(num_bits)?;
        self.state = new_bits;
        Ok(num_bits)
    }

    pub fn next_state(
        &mut self,
        br: &mut BitReaderReversed<'_>,
    ) -> Result<u8, HuffmanDecoderError> {
        let num_bits = self.table.decode[self.state as usize].num_bits;
        let new_bits = br.get_bits(num_bits)?;
        self.state <<= num_bits;
        self.state &= self.table.decode.len() as u64 - 1;
        self.state |= new_bits;
        Ok(num_bits)
    }
}

impl Default for HuffmanTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HuffmanTable {
    pub fn new() -> HuffmanTable {
        HuffmanTable {
            decode: Vec::new(),

            weights: Vec::with_capacity(256),
            max_num_bits: 0,
            bits: Vec::with_capacity(256),
            bit_ranks: Vec::with_capacity(11),
            rank_indexes: Vec::with_capacity(11),
            fse_table: FSETable::new(),
        }
    }

    pub fn reinit_from(&mut self, other: &Self) {
        self.reset();
        self.decode.extend_from_slice(&other.decode);
        self.weights.extend_from_slice(&other.weights);
        self.max_num_bits = other.max_num_bits;
        self.bits.extend_from_slice(&other.bits);
        self.rank_indexes.extend_from_slice(&other.rank_indexes);
        self.fse_table.reinit_from(&other.fse_table);
    }

    pub fn reset(&mut self) {
        self.decode.clear();
        self.weights.clear();
        self.max_num_bits = 0;
        self.bits.clear();
        self.bit_ranks.clear();
        self.rank_indexes.clear();
        self.fse_table.reset();
    }

    pub fn build_decoder(&mut self, source: &[u8]) -> Result<u32, HuffmanTableError> {
        self.decode.clear();

        let bytes_used = self.read_weights(source)?;
        self.build_table_from_weights()?;
        Ok(bytes_used)
    }

    fn read_weights(&mut self, source: &[u8]) -> Result<u32, HuffmanTableError> {
        use HuffmanTableError as err;

        if source.is_empty() {
            return Err(err::SourceIsEmpty);
        }
        let header = source[0];
        let mut bits_read = 8;

        match header {
            0..=127 => {
                let fse_stream = &source[1..];
                if header as usize > fse_stream.len() {
                    return Err(err::NotEnoughBytesForWeights {
                        got_bytes: fse_stream.len(),
                        expected_bytes: header,
                    });
                }
                //fse decompress weights
                let bytes_used_by_fse_header = self
                    .fse_table
                    .build_decoder(fse_stream, /*TODO find actual max*/ 100)?;

                if bytes_used_by_fse_header > header as usize {
                    return Err(err::FSETableUsedTooManyBytes {
                        used: bytes_used_by_fse_header,
                        available_bytes: header,
                    });
                }

                vprintln!(
                    "Building fse table for huffman weights used: {}",
                    bytes_used_by_fse_header
                );
                let mut dec1 = FSEDecoder::new(&self.fse_table);
                let mut dec2 = FSEDecoder::new(&self.fse_table);

                let compressed_start = bytes_used_by_fse_header;
                let compressed_length = header as usize - bytes_used_by_fse_header;

                let compressed_weights = &fse_stream[compressed_start..];
                if compressed_weights.len() < compressed_length {
                    return Err(err::NotEnoughBytesToDecompressWeights {
                        have: compressed_weights.len(),
                        need: compressed_length,
                    });
                }
                let compressed_weights = &compressed_weights[..compressed_length];
                let mut br = BitReaderReversed::new(compressed_weights);

                bits_read += (bytes_used_by_fse_header + compressed_length) * 8;

                //skip the 0 padding at the end of the last byte of the bit stream and throw away the first 1 found
                let mut skipped_bits = 0;
                loop {
                    let val = br.get_bits(1)?;
                    skipped_bits += 1;
                    if val == 1 || skipped_bits > 8 {
                        break;
                    }
                }
                if skipped_bits > 8 {
                    //if more than 7 bits are 0, this is not the correct end of the bitstream. Either a bug or corrupted data
                    return Err(err::ExtraPadding { skipped_bits });
                }

                dec1.init_state(&mut br)?;
                dec2.init_state(&mut br)?;

                self.weights.clear();

                loop {
                    let w = dec1.decode_symbol();
                    self.weights.push(w);
                    dec1.update_state(&mut br)?;

                    if br.bits_remaining() <= -1 {
                        //collect final states
                        self.weights.push(dec2.decode_symbol());
                        break;
                    }

                    let w = dec2.decode_symbol();
                    self.weights.push(w);
                    dec2.update_state(&mut br)?;

                    if br.bits_remaining() <= -1 {
                        //collect final states
                        self.weights.push(dec1.decode_symbol());
                        break;
                    }
                    //maximum number of weights is 255 because we use u8 symbols and the last weight is inferred from the sum of all others
                    if self.weights.len() > 255 {
                        return Err(err::TooManyWeights {
                            got: self.weights.len(),
                        });
                    }
                }
            }
            _ => {
                // weights are directly encoded
                let weights_raw = &source[1..];
                let num_weights = header - 127;
                self.weights.resize(num_weights as usize, 0);

                let bytes_needed = if num_weights % 2 == 0 {
                    num_weights as usize / 2
                } else {
                    (num_weights as usize / 2) + 1
                };

                if weights_raw.len() < bytes_needed {
                    return Err(err::NotEnoughBytesInSource {
                        got: weights_raw.len(),
                        need: bytes_needed,
                    });
                }

                for idx in 0..num_weights {
                    if idx % 2 == 0 {
                        self.weights[idx as usize] = weights_raw[idx as usize / 2] >> 4;
                    } else {
                        self.weights[idx as usize] = weights_raw[idx as usize / 2] & 0xF;
                    }
                    bits_read += 4;
                }
            }
        }

        let bytes_read = if bits_read % 8 == 0 {
            bits_read / 8
        } else {
            (bits_read / 8) + 1
        };
        Ok(bytes_read as u32)
    }

    fn build_table_from_weights(&mut self) -> Result<(), HuffmanTableError> {
        use HuffmanTableError as err;

        self.bits.clear();
        self.bits.resize(self.weights.len() + 1, 0);

        let mut weight_sum: u32 = 0;
        for w in &self.weights {
            if *w > MAX_MAX_NUM_BITS {
                return Err(err::WeightBiggerThanMaxNumBits { got: *w });
            }
            weight_sum += if *w > 0 { 1_u32 << (*w - 1) } else { 0 };
        }

        if weight_sum == 0 {
            return Err(err::MissingWeights);
        }

        let max_bits = highest_bit_set(weight_sum) as u8;
        let left_over = (1 << max_bits) - weight_sum;

        //left_over must be power of two
        if !left_over.is_power_of_two() {
            return Err(err::LeftoverIsNotAPowerOf2 { got: left_over });
        }

        let last_weight = highest_bit_set(left_over) as u8;

        for symbol in 0..self.weights.len() {
            let bits = if self.weights[symbol] > 0 {
                max_bits + 1 - self.weights[symbol]
            } else {
                0
            };
            self.bits[symbol] = bits;
        }

        self.bits[self.weights.len()] = max_bits + 1 - last_weight;
        self.max_num_bits = max_bits;

        if max_bits > MAX_MAX_NUM_BITS {
            return Err(err::MaxBitsTooHigh { got: max_bits });
        }

        self.bit_ranks.clear();
        self.bit_ranks.resize((max_bits + 1) as usize, 0);
        for num_bits in &self.bits {
            self.bit_ranks[(*num_bits) as usize] += 1;
        }

        //fill with dummy symbols
        self.decode.resize(
            1 << self.max_num_bits,
            Entry {
                symbol: 0,
                num_bits: 0,
            },
        );

        //starting codes for each rank
        self.rank_indexes.clear();
        self.rank_indexes.resize((max_bits + 1) as usize, 0);

        self.rank_indexes[max_bits as usize] = 0;
        for bits in (1..self.rank_indexes.len() as u8).rev() {
            self.rank_indexes[bits as usize - 1] = self.rank_indexes[bits as usize]
                + self.bit_ranks[bits as usize] as usize * (1 << (max_bits - bits));
        }

        assert!(
            self.rank_indexes[0] == self.decode.len(),
            "rank_idx[0]: {} should be: {}",
            self.rank_indexes[0],
            self.decode.len()
        );

        for symbol in 0..self.bits.len() {
            let bits_for_symbol = self.bits[symbol];
            if bits_for_symbol != 0 {
                // allocate code for the symbol and set in the table
                // a code ignores all max_bits - bits[symbol] bits, so it gets
                // a range that spans all of those in the decoding table
                let base_idx = self.rank_indexes[bits_for_symbol as usize];
                let len = 1 << (max_bits - bits_for_symbol);
                self.rank_indexes[bits_for_symbol as usize] += len;
                for idx in 0..len {
                    self.decode[base_idx + idx].symbol = symbol as u8;
                    self.decode[base_idx + idx].num_bits = bits_for_symbol;
                }
            }
        }

        Ok(())
    }
}
