from pathlib import Path

def edit(p,a,b):
 p=Path(p);s=p.read_text();assert a in s,(p,a[:60]);p.write_text(s.replace(a,b))
edit('crates/strata-input/src/parse.rs','headers.len() != schema.len()','headers.len() > schema.len()')
# Protected metadata in new format 0.2; old 0.1 remains readable.
edit('crates/strata-format/src/version.rs','major: 0, minor: 1 };','major: 0, minor: 2 };')
p=Path('crates/strata-storage/src/io.rs');s=p.read_text();s+='''
/// CRC32 of the file prefix up to its current position, without allocating the file.
/// Leaves the cursor at the original position.
pub fn checksum_prefix(file: &mut std::fs::File, path: &Path) -> Result<u32> {
    use std::io::{Read, Seek, SeekFrom};
    let run = || -> std::io::Result<u32> {
        let end=file.stream_position()?;file.seek(SeekFrom::Start(0))?;
        let mut left=end;let mut buffer=[0u8;65536];let mut hash=crc32fast::Hasher::new();
        while left>0 {let count=(left as usize).min(buffer.len());file.read_exact(&mut buffer[..count])?;hash.update(&buffer[..count]);left-=count as u64;}
        Ok(hash.finalize())
    };
    let mut run=run;run().map_err(|source|Error::Io{path:path.to_path_buf(),source})
}
''';p.write_text(s)
p=Path('crates/strata-storage/src/segment/writer.rs');s=p.read_text().replace('            .write(true)\n            .create(true)','            .read(true)\n            .write(true)\n            .create(true)');s=s.replace('        let blocks_crc = self.blocks_hasher.clone().finalize();','        let mut blocks_crc = self.blocks_hasher.clone().finalize();');s=s.replace('        write_u32(&mut self.file, &self.path, blocks_crc)?;','        blocks_crc = crate::io::checksum_prefix(&mut self.file,&self.path)?;\n        self.meta.footer.checksum = Some(u64::from(blocks_crc));\n        write_u32(&mut self.file, &self.path, blocks_crc)?;');p.write_text(s)
p=Path('crates/strata-storage/src/segment/reader.rs');s=p.read_text().replace('    let actual_crc = strata_format::crc32(blocks_region);','    let actual_crc = strata_format::crc32(if minor >= 2 { &bytes[..bytes.len()-8] } else { blocks_region });');s=s.replace('    let segment_id = SegmentId::new(cur.read_string()?);','    schema.validate()?;\n    let segment_id = SegmentId::new(cur.read_string()?);');s=s.replace('    if directory_offset as usize > footer_start {','    if directory_offset > footer_start as u64 {');s=s.replace('        let id = BlockId(dcur.read_u32()?);','        let id = BlockId(dcur.read_u32()?);\n        if id.0 as usize != blocks.len() { return Err(Error::corrupt(path, "duplicate or nonsequential block id")); }');s=s.replace('        for _ in 0..column_count {\n            let field_name = dcur.read_string()?;','        for col_index in 0..column_count {\n            let field_name = dcur.read_string()?;\n            if field_name != schema.fields[col_index].name { return Err(Error::corrupt(path,"directory column name mismatch")); }');p.write_text(s)
p=Path('crates/strata-storage/src/recover.rs');s=p.read_text().replace('            .write(true)\n            .create(true)','            .read(true)\n            .write(true)\n            .create(true)');s=s.replace('        write_bytes(&mut file, &tmp, header_bytes)?;','        let mut header_bytes=header_bytes.to_vec();\n        header_bytes[10..12].copy_from_slice(&FORMAT_VERSION.minor.to_le_bytes());\n        write_bytes(&mut file, &tmp, &header_bytes)?;');s=s.replace('        let blocks_crc = hasher.finalize();','        let _ = hasher.finalize();');s=s.replace('        write_u32(&mut file, &tmp, blocks_crc)?;','        let checksum=crate::io::checksum_prefix(&mut file,&tmp)?;\n        write_u32(&mut file, &tmp, checksum)?;');s=s.replace('        let records = reader.read_all_records()?;','        let mut records=0u64;\n        for block in 0..reader.block_count() { records += reader.read_block(block)?.len() as u64; }').replace('            record_count: records.len() as u64,','            record_count: records,');p.write_text(s)
# Bound all externally supplied metadata before heap allocation.
p=Path('crates/strata-storage/src/io.rs');s=p.read_text().replace('    std::fs::read(path).map_err(|source| Error::Io {','''    let length=std::fs::metadata(path).map_err(|source|Error::Io{path:path.to_path_buf(),source})?.len();
    if length > 1024 * 1024 * 1024 { return Err(Error::corrupt(path,"file exceeds 1 GiB read limit")); }
    std::fs::read(path).map_err(|source| Error::Io {''');p.write_text(s)
p=Path('crates/strata-storage/src/dataset/reader.rs');s=p.read_text();s=s.replace('        let text = std::fs::read_to_string(&manifest_path).map_err(|source| Error::Io {','        if std::fs::metadata(&manifest_path).map_err(|source|Error::Io{path:manifest_path.clone(),source})?.len()>16*1024*1024 { return Err(Error::corrupt(&manifest_path,"manifest exceeds 16 MiB")); }\n        let text = std::fs::read_to_string(&manifest_path).map_err(|source| Error::Io {');s=s.replace('        Ok(Self {','        manifest.schema.validate()?;\n        Ok(Self {',1);s=s.replace('        SegmentReader::open_with_mode(path, mode)','''        let canonical=path.path.canonicalize().map_err(|source|Error::Io{path:path.path.clone(),source})?;
        let root=self.layout.segments_dir().canonicalize().map_err(|source|Error::Io{path:self.layout.segments_dir(),source})?;
        if !canonical.starts_with(&root) { return Err(Error::corrupt(&path.path,"segment symlink escapes dataset")); }
        let segment=SegmentReader::open_with_mode(path, mode)?;
        if segment.schema()!=self.schema() || segment.meta().header.record_count!=entry.record_count || segment.block_count()!=entry.block_count as usize { return Err(Error::corrupt(segment.path(),"segment does not match manifest")); }
        Ok(segment)''');p.write_text(s)
p=Path('crates/strata-storage/src/block/reader.rs');s=p.read_text().replace('    let column_count = cur.read_u32()? as usize;','    let column_count = cur.read_u32()? as usize;\n    if (row_count as usize).checked_mul(column_count).is_none_or(|n|n>4_194_304) { return Err(Error::corrupt(path,"block has too many cells")); }');p.write_text(s)
p=Path('crates/strata-codec/src/pipeline.rs');s=p.read_text().replace('    let present = stream_to_values(logical_type, stream)?;','''    let present = stream_to_values(logical_type, stream)?;
    for value in &present {
        match (logical_type,value) {
            (LogicalType::UnsignedInteger(t),FieldValue::UnsignedInteger(n)) => check_u_width(*n,t.width)?,
            (LogicalType::SignedInteger(t),FieldValue::SignedInteger(n)) => check_i_width(*n,t.width)?,
            (LogicalType::FixedDecimal(t),FieldValue::FixedDecimal(n)) => check_i_width(*n,t.width)?,
            (LogicalType::Enum(t),FieldValue::Enum(s)) if !t.variants.contains(s) => return Err(Error::Decode{message:"enum outside schema domain".into()}),
            (LogicalType::CategoricalString(t),FieldValue::CategoricalString(s)) if t.max_bytes.is_some_and(|max|s.len()>max as usize) => return Err(Error::Decode{message:"string exceeds schema bound".into()}),
            (LogicalType::Binary(t),FieldValue::Binary(s)) if t.max_bytes.is_some_and(|max|s.len()>max as usize) => return Err(Error::Decode{message:"binary exceeds schema bound".into()}),
            _=>{}
        }
    }''');p.write_text(s)
# Avoid compiler-eliminated microbench output and invented logical byte for all-null columns.
p=Path('crates/strata-optimize/src/bench.rs');s=p.read_text().replace('first.logical_bytes.max(1)','first.logical_bytes').replace('.map(|_| ())','.map(|v| { std::hint::black_box(v); })');p.write_text(s)
# Missing index operations must not imply no matching records.
p=Path('crates/strata-query/src/exec.rs');s=p.read_text();s=s.replace('                let pruned = prune_with_indexes(&indexes, schema, predicates, &all, &mut notes)?;','''                let pruned=match prune_with_indexes(&indexes,schema,predicates,&all,&mut notes) { Ok(v)=>v,Err(e)=>{used_indexes=false;notes.push(format!("unusable index; full scan: {e}"));all.clone()} };''');s=s.replace('indexes.lookup_categorical(field, v).unwrap_or_default()','indexes.lookup_categorical(field, v)?').replace('.prune_unsigned_range(field, lo, hi)\n                .unwrap_or_default()','.prune_unsigned_range(field, lo, hi)?').replace('.prune_signed_range(field, lo, hi)\n                .unwrap_or_default()','.prune_signed_range(field, lo, hi)?').replace('.prune_unsigned_range(field, lo, u64::MAX)\n                    .unwrap_or_default()','.prune_unsigned_range(field, lo, u64::MAX)?').replace('.prune_unsigned_range(field, 0, hi)\n                    .unwrap_or_default()','.prune_unsigned_range(field, 0, hi)?').replace('indexes.lookup_sequence(field, *seq).unwrap_or_default()','indexes.lookup_sequence(field, *seq)?');s=s.replace('''.unwrap_or_else(|_| {
                    indexes
                        .prune_signed_range(field, lo, hi)
                        .unwrap_or_default()
                })''','''.or_else(|_| indexes.prune_signed_range(field, lo, hi))?''');s=s.replace('                    .unwrap_or_default();','                    ?;');p.write_text(s)
# Eliminate obsolete CLI imports; keep shared service boundary.
p=Path('crates/strata-cli/src/main.rs');s=p.read_text().replace('use std::time::Instant;\n','').replace('compressor_for, resolve_profile_plans, CompressionCodec, EncodingPlan','resolve_profile_plans, EncodingPlan').replace('load_profile, CompressionCodecName, CompressionProfile, DurabilityMode','load_profile, CompressionProfile').replace('DatasetReader, DatasetWriterOptions, IngestPipeline,','DatasetReader,').replace('    PipelineOptions, SegmentReadMode,','    SegmentReadMode,');a=s.find('\nfn map_codec(');b=s.find('\nfn ',a+2);s=s[:a]+(s[b:] if b!=-1 else '') if a!=-1 else s;p.write_text(s)
