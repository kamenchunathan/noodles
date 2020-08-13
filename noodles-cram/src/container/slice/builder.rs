use std::{collections::HashMap, io};

use crate::{
    container::{
        block, compression_header::data_series_encoding_map::DataSeries, Block, CompressionHeader,
        ReferenceSequenceId,
    },
    writer, BitWriter, Record,
};

use super::{Header, Slice};

use noodles_bam as bam;

const CORE_DATA_BLOCK_CONTENT_ID: i32 = 0;

#[derive(Debug)]
pub struct Builder {
    reference_sequence_id: ReferenceSequenceId,
    records: Vec<Record>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddRecordError {
    ReferenceSequenceIdMismatch,
}

impl Builder {
    pub fn new(reference_sequence_id: ReferenceSequenceId) -> Self {
        Self {
            reference_sequence_id,
            records: Vec::new(),
        }
    }

    pub fn add_record(&mut self, record: Record) -> Result<&Record, AddRecordError> {
        let record_reference_sequence_id =
            bam::record::ReferenceSequenceId::from(record.reference_id);

        match self.reference_sequence_id {
            ReferenceSequenceId::None => match *record_reference_sequence_id {
                Some(_) => Err(AddRecordError::ReferenceSequenceIdMismatch),
                None => {
                    self.records.push(record);
                    Ok(self.records.last().unwrap())
                }
            },
            ReferenceSequenceId::Some(slice_reference_sequence_id) => {
                match *record_reference_sequence_id {
                    Some(id) => {
                        if slice_reference_sequence_id == id {
                            self.records.push(record);
                            Ok(self.records.last().unwrap())
                        } else {
                            Err(AddRecordError::ReferenceSequenceIdMismatch)
                        }
                    }
                    None => Err(AddRecordError::ReferenceSequenceIdMismatch),
                }
            }
            ReferenceSequenceId::Many => todo!(),
        }
    }

    pub fn build(self, compression_header: &CompressionHeader) -> io::Result<Slice> {
        let alignment_start = self
            .records
            .first()
            .map(|r| r.alignment_start())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no records in builder"))?;

        let mut core_data_writer = BitWriter::new(Vec::new());

        let mut external_data_writers = HashMap::new();

        for i in 0..DataSeries::LEN {
            let block_content_id = (i + 1) as i32;
            external_data_writers.insert(block_content_id, Vec::new());
        }

        for &block_content_id in compression_header.tag_encoding_map().keys() {
            external_data_writers.insert(block_content_id, Vec::new());
        }

        let mut record_writer = writer::record::Writer::new(
            compression_header,
            &mut core_data_writer,
            &mut external_data_writers,
            self.reference_sequence_id,
            alignment_start,
        );

        for record in &self.records {
            record_writer.write_record(record)?;
        }

        let core_data_block = core_data_writer.finish().map(|buf| {
            Block::new(
                block::CompressionMethod::None,
                block::ContentType::CoreData,
                CORE_DATA_BLOCK_CONTENT_ID,
                buf.len() as i32,
                buf,
                0,
            )
        })?;

        let mut block_content_ids = vec![CORE_DATA_BLOCK_CONTENT_ID];

        for &block_content_id in external_data_writers.keys() {
            block_content_ids.push(block_content_id);
        }

        let external_blocks: Vec<_> = external_data_writers
            .into_iter()
            .map(|(block_content_id, buf)| {
                Block::new(
                    block::CompressionMethod::None,
                    block::ContentType::ExternalData,
                    block_content_id,
                    buf.len() as i32,
                    buf,
                    0,
                )
            })
            .collect();

        // TODO
        let header = Header::new(
            self.reference_sequence_id,
            1,
            0,
            self.records.len() as i32,
            0,
            (external_blocks.len() + 1) as i32,
            block_content_ids,
            0,
            Default::default(),
            Vec::new(),
        );

        Ok(Slice::new(header, core_data_block, external_blocks))
    }
}
