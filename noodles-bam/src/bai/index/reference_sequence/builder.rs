use std::{cmp, collections::HashMap};

use noodles_bgzf as bgzf;

use crate::Record;

use super::{
    bin::{self, Chunk},
    Bin, Metadata, ReferenceSequence,
};

const WINDOW_SIZE: i32 = 16384;

// § 5.2 The BAI index format for BAM files (2020-07-19)
const MAX_INTERVAL_COUNT: usize = 131072;

#[derive(Debug)]
pub struct Builder {
    bin_builders: HashMap<u32, bin::Builder>,
    intervals: Vec<bgzf::VirtualPosition>,
    start_position: bgzf::VirtualPosition,
    end_position: bgzf::VirtualPosition,
    mapped_record_count: u64,
    unmapped_record_count: u64,
}

impl Builder {
    pub fn add_record(&mut self, record: &Record, chunk: Chunk) {
        self.update_bins(record, chunk);
        self.update_linear_index(record, chunk);
        self.update_metadata(record, chunk);
    }

    pub fn build(self) -> ReferenceSequence {
        if self.bin_builders.is_empty() {
            return ReferenceSequence::default();
        }

        let bins: Vec<_> = self
            .bin_builders
            .into_iter()
            .map(|(_, b)| b.build())
            .collect();

        let metadata = Metadata::new(
            self.start_position,
            self.end_position,
            self.mapped_record_count,
            self.unmapped_record_count,
        );

        ReferenceSequence::new(bins, self.intervals, Some(metadata))
    }

    fn update_bins(&mut self, record: &Record, chunk: Chunk) {
        let bin_id = record.bin() as u32;

        let builder = self.bin_builders.entry(bin_id).or_insert_with(|| {
            let mut builder = Bin::builder();
            builder.set_id(bin_id);
            builder
        });

        builder.add_chunk(chunk);
    }

    fn update_linear_index(&mut self, record: &Record, chunk: Chunk) {
        let start = i32::from(record.position());
        let reference_len = record.cigar().reference_len() as i32;
        let end = start + reference_len - 1;

        let linear_index_start_offset = ((start - 1) / WINDOW_SIZE) as usize;
        let linear_index_end_offset = ((end - 1) / WINDOW_SIZE) as usize;

        for i in linear_index_start_offset..=linear_index_end_offset {
            self.intervals[i] = chunk.start();
        }
    }

    fn update_metadata(&mut self, record: &Record, chunk: Chunk) {
        if record.flags().is_unmapped() {
            self.unmapped_record_count += 1;
        } else {
            self.mapped_record_count += 1;
        }

        self.start_position = cmp::min(self.start_position, chunk.start());
        self.end_position = cmp::max(self.end_position, chunk.end());
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            bin_builders: HashMap::new(),
            intervals: vec![bgzf::VirtualPosition::default(); MAX_INTERVAL_COUNT],
            start_position: bgzf::VirtualPosition::max(),
            end_position: bgzf::VirtualPosition::default(),
            mapped_record_count: 0,
            unmapped_record_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use noodles_sam::{
        self as sam,
        header::ReferenceSequences,
        record::{Flags, Position},
    };

    use super::*;

    #[test]
    fn test_build() -> Result<(), Box<dyn std::error::Error>> {
        let mut reference_sequences = ReferenceSequences::default();
        reference_sequences.insert(
            String::from("sq0"),
            sam::header::ReferenceSequence::new(String::from("sq0"), 8),
        );

        let mut builder = Builder::default();

        let record = Record::try_from_sam_record(
            &reference_sequences,
            &sam::Record::builder()
                .set_flags(Flags::empty())
                .set_position(Position::from(2))
                .set_cigar("4M".parse()?)
                .build(),
        )?;

        builder.add_record(
            &record,
            Chunk::new(
                bgzf::VirtualPosition::from(55),
                bgzf::VirtualPosition::from(89),
            ),
        );

        let record = Record::try_from_sam_record(
            &reference_sequences,
            &sam::Record::builder()
                .set_position(Position::from(6))
                .set_cigar("2M".parse()?)
                .build(),
        )?;

        builder.add_record(
            &record,
            Chunk::new(
                bgzf::VirtualPosition::from(89),
                bgzf::VirtualPosition::from(144),
            ),
        );

        let actual = builder.build();

        let mut expected_linear_index = vec![bgzf::VirtualPosition::default(); MAX_INTERVAL_COUNT];
        expected_linear_index[0] = bgzf::VirtualPosition::from(89);

        let expected = ReferenceSequence::new(
            vec![Bin::new(
                4681,
                vec![Chunk::new(
                    bgzf::VirtualPosition::from(55),
                    bgzf::VirtualPosition::from(144),
                )],
            )],
            expected_linear_index,
            Some(Metadata::new(
                bgzf::VirtualPosition::from(55),
                bgzf::VirtualPosition::from(144),
                1,
                1,
            )),
        );

        assert_eq!(actual, expected);

        Ok(())
    }

    #[test]
    fn test_build_with_no_bins() {
        let reference_sequence = Builder::default().build();
        assert_eq!(reference_sequence, ReferenceSequence::default());
    }

    #[test]
    fn test_default() {
        let builder = Builder::default();

        assert!(builder.bin_builders.is_empty());

        assert!(builder
            .intervals
            .iter()
            .all(|&pos| pos == bgzf::VirtualPosition::default()));

        assert_eq!(builder.start_position, bgzf::VirtualPosition::max());
        assert_eq!(builder.end_position, bgzf::VirtualPosition::default());
        assert_eq!(builder.mapped_record_count, 0);
        assert_eq!(builder.unmapped_record_count, 0);
    }
}
