from pathlib import Path

def edit(p,a,b):
 p=Path(p); s=p.read_text(); assert a in s,(p,a[:80]); p.write_text(s.replace(a,b))
base='crates/strata-codec/src/'
Path(base+'limits.rs').write_text('''//! Resource limits for untrusted codec payloads.
use crate::error::{Error, Result};
/// Maximum values in one decoded column (also enforced by writers).
pub const MAX_VALUES: usize = 1_048_576;
/// Maximum expanded byte payload for one codec operation.
pub const MAX_DECODE_BYTES: usize = 256 * 1024 * 1024;
/// Reject untrusted counts before allocation.
pub fn check_count(count: usize) -> Result<()> {
    if count > MAX_VALUES { return Err(Error::Decode { message: "value count exceeds safety limit".into() }); }
    Ok(())
}
/// Reject untrusted decompressed sizes before allocation.
pub fn check_bytes(count: usize) -> Result<()> {
    if count > MAX_DECODE_BYTES { return Err(Error::Decode { message: "decoded bytes exceed safety limit".into() }); }
    Ok(())
}
''')
edit(base+'lib.rs','pub mod compress;', 'pub mod limits;\npub mod compress;')
edit(base+'codecs/varint.rs','    let mut pos = 4usize;', '''    crate::limits::check_count(len)?;
    if len > bytes.len() - 4 { return Err(Error::Decode { message: "varint count exceeds payload".into() }); }
    let mut pos = 4usize;''')
edit(base+'codecs/varint.rs','if shift >= 64 {','if shift >= 64 || (shift == 63 && b & 0x7f > 1) {')
edit(base+'codecs/bit_packing.rs','    if bit_width == 0 {\n        return Ok(Vec::new());','    if bit_width == 0 {\n        if !values.is_empty() { return Err(Error::InvalidInput { message: "zero width for nonempty input".into() }); }\n        return Ok(Vec::new());')
edit(base+'codecs/bit_packing.rs','    if meta.blob.len() < 5 {','    if meta.blob.len() != 5 {')
edit(base+'codecs/bit_packing.rs','pub fn unpack(bytes: &[u8], count: usize, bit_width: u8) -> Result<Vec<u64>> {','''pub fn unpack(bytes: &[u8], count: usize, bit_width: u8) -> Result<Vec<u64>> {
    crate::limits::check_count(count)?;
    if bit_width > 64 { return Err(Error::Decode { message: "bit width exceeds 64".into() }); }''')
edit(base+'codecs/bit_packing.rs','if count == 0 {','if count == 0 && bytes.is_empty() {')
edit(base+'codecs/bit_packing.rs','if bytes.len() < need {','if bytes.len() != need {')
edit(base+'codecs/dictionary.rs','    let mut pos = 4usize;', '''    crate::limits::check_count(n)?;
    if n > (blob.len() - 4) / 4 { return Err(Error::Decode { message: "dictionary count exceeds metadata".into() }); }
    let mut pos = 4usize;''')
edit(base+'codecs/dictionary.rs','    for id in ids {','    let mut expanded = 0usize;\n    for id in ids {')
edit(base+'codecs/dictionary.rs','        values.push(entry.clone());','        expanded = expanded.checked_add(entry.len()).ok_or_else(|| Error::Decode { message: "dictionary expansion overflow".into() })?;\n        crate::limits::check_bytes(expanded)?;\n        values.push(entry.clone());')
edit(base+'codecs/enum_bitpack.rs','    let mut pos = 9usize;', '''    crate::limits::check_count(n_variants)?;
    if n_variants > (meta.blob.len() - 9) / 4 { return Err(Error::Decode { message: "enum count exceeds metadata".into() }); }
    let mut pos = 9usize;''')
edit(base+'codecs/enum_bitpack.rs','    for id in ids {','    let mut expanded = 0usize;\n    for id in ids {')
edit(base+'codecs/enum_bitpack.rs','        values.push(v.clone());','        expanded = expanded.checked_add(v.len()).ok_or_else(|| Error::Decode { message: "enum expansion overflow".into() })?;\n        crate::limits::check_bytes(expanded)?;\n        values.push(v.clone());')
edit(base+'codecs/plain.rs','    match tag {','''    crate::limits::check_count(len)?;
    let width = match tag { 1 | 2 => 8, 3 => 1, 4 => 4, _ => 1 };
    if len > bytes.len().saturating_sub(5) / width { return Err(Error::Decode { message: "plain count exceeds payload".into() }); }
    match tag {''')
edit(base+'codecs/plain.rs','                values.push(cur.read_u8()? != 0);','''                let value = cur.read_u8()?;
                if value > 1 { return Err(Error::Decode { message: "invalid boolean".into() }); }
                values.push(value == 1);''')
edit(base+'codecs/rle.rs','        for _ in 0..count {','''        if count == 0 { return Err(Error::Decode { message: "zero RLE run".into() }); }
        crate::limits::check_count(values.len().checked_add(count as usize).ok_or_else(|| Error::Decode { message: "RLE overflow".into() })?)?;
        for _ in 0..count {''')
edit(base+'codecs/boolean_bitpack.rs','    let need = count.div_ceil(8);','    crate::limits::check_count(count)?;\n    let need = count.div_ceil(8);')
edit(base+'codecs/boolean_bitpack.rs','if bytes.len() < need {','if bytes.len() != need {')
edit(base+'nulls.rs','    let mut out = Vec::with_capacity(row_count);','''    crate::limits::check_count(row_count)?;
    if bitmap.len() != row_count.div_ceil(8) { return Err(Error::Decode { message: "invalid null bitmap length".into() }); }
    let mut out = Vec::with_capacity(row_count);''')
edit(base+'pipeline.rs','    plan.validate()?;','    crate::limits::check_count(values.len())?;\n    plan.validate()?;')
edit(base+'pipeline.rs','    let mut metas = Vec::with_capacity(n_meta);','''    if n_meta > plan.steps.len() || n_meta > bytes.len().saturating_sub(pos) / 5 { return Err(Error::Decode { message: "invalid metadata count".into() }); }
    let mut metas = Vec::with_capacity(n_meta);''')
edit(base+'pipeline.rs','    let present = stream_to_values(logical_type, stream)?;','''    if meta_idx != 0 { return Err(Error::Decode { message: "unused step metadata".into() }); }
    let present = stream_to_values(logical_type, stream)?;''')
edit(base+'compress.rs','        lz4_flex::block::decompress(input, uncompressed_len).map_err(|e| Error::Decode {','        crate::limits::check_bytes(uncompressed_len)?;\n        let out = lz4_flex::block::decompress(input, uncompressed_len).map_err(|e| Error::Decode {')
edit(base+'compress.rs','            message: format!("lz4 decompress failed: {e}"),\n        })','''            message: format!("lz4 decompress failed: {e}"),
        })?;
        if out.len() != uncompressed_len { return Err(Error::Decode { message: "lz4 length mismatch".into() }); }
        Ok(out)''')
edit(base+'compress.rs','        zstd::bulk::decompress(input, uncompressed_len).map_err(|e| Error::Decode {','        crate::limits::check_bytes(uncompressed_len)?;\n        let out = zstd::bulk::decompress(input, uncompressed_len).map_err(|e| Error::Decode {')
edit(base+'compress.rs','            message: format!("zstd decompress failed: {e}"),\n        })','''            message: format!("zstd decompress failed: {e}"),
        })?;
        if out.len() != uncompressed_len { return Err(Error::Decode { message: "zstd length mismatch".into() }); }
        Ok(out)''')
# Actual sample only, no pattern replication.
p=Path('crates/strata-optimize/src/report.rs');s=p.read_text(); a=s.index('    if records.is_empty() || target_rows == 0 {',s.index('fn build_sample')); b=s.index('\n}\n',a);s=s[:a]+'    records[..records.len().min(target_rows)].to_vec()'+s[b:];p.write_text(s)
# Allocation/offset/manifest hardening.
b='crates/strata-storage/src/'
edit(b+'segment/reader.rs','    let blocks_region = &bytes[header_end..directory_offset as usize];','''    if directory_offset < header_end as u64 { return Err(Error::corrupt(path, "directory overlaps header")); }
    let blocks_region = &bytes[header_end..directory_offset as usize];''')
edit(b+'segment/reader.rs','    let mut blocks = Vec::with_capacity(block_count as usize);','''    if block_count as usize > dcur.remaining() / 28 { return Err(Error::corrupt(path, "block count exceeds directory")); }
    let mut blocks = Vec::with_capacity(block_count as usize);
    let mut expected_offset = header_end as u64;
    let mut total_rows = 0u64;''')
edit(b+'segment/reader.rs','        let mut columns = Vec::with_capacity(column_count);','''        if column_count != schema.len() || column_count > dcur.remaining() / 21 { return Err(Error::corrupt(path, "invalid column count")); }
        if file_offset != expected_offset || block_length < 14 || file_offset.checked_add(block_length).filter(|e| *e <= directory_offset).is_none() { return Err(Error::corrupt(path, "invalid block range")); }
        expected_offset = file_offset + block_length;
        total_rows += u64::from(row_count);
        let mut columns = Vec::with_capacity(column_count);''')
edit(b+'segment/reader.rs','            logical += logical_bytes;','            logical = logical.checked_add(logical_bytes).ok_or_else(|| Error::corrupt(path, "logical byte overflow"))?;')
edit(b+'segment/reader.rs','            semantic_encoded_bytes: 0,\n            compressed_bytes: block_length,\n            compression_codec: 0,','''            semantic_encoded_bytes: u64::from(u32::from_le_bytes(bytes[file_offset as usize + 2..file_offset as usize + 6].try_into().unwrap())),
            compressed_bytes: u64::from(u32::from_le_bytes(bytes[file_offset as usize + 6..file_offset as usize + 10].try_into().unwrap())),
            compression_codec: bytes[file_offset as usize + 1],''')
edit(b+'segment/reader.rs','    if blocks.len() as u32 != block_count {','''    if dcur.remaining() != 0 || expected_offset != directory_offset || total_rows != record_count { return Err(Error::corrupt(path, "inconsistent segment directory totals")); }
    if blocks.len() as u32 != block_count {''')
edit(b+'dataset/reader.rs','        Ok(Self {','''        if manifest.manifest_version == 0 || manifest.manifest_version > DatasetManifest::VERSION || !strata_format::FORMAT_VERSION.is_compatible_with(manifest.format_version) { return Err(Error::corrupt(&manifest_path, "unsupported manifest version")); }
        let mut names = std::collections::HashSet::new();
        for segment in &manifest.segments {
            if segment.file.is_empty() || Path::new(&segment.file).components().count() != 1 || !matches!(Path::new(&segment.file).components().next(), Some(std::path::Component::Normal(_))) || !names.insert(&segment.file) { return Err(Error::corrupt(&manifest_path, "invalid or duplicate segment filename")); }
        }
        Ok(Self {''')
edit(b+'dataset/reader.rs','        let mut records = Vec::with_capacity(self.manifest.record_count as usize);','        let mut records = Vec::new();')
edit(b+'block/reader.rs','    let row_count = cur.read_u32()?;','    let row_count = cur.read_u32()?;\n    strata_codec::limits::check_count(row_count as usize)?;')
edit(b+'block/reader.rs','    let decode_names: Vec<String> = match project {','''    let mut end_offset = cur.position();
    for column in &col_metas {
        if column.data_rel_offset as usize != end_offset { return Err(Error::corrupt(path, "column ranges overlap or contain gaps")); }
        end_offset = end_offset.checked_add(column.data_len as usize).ok_or_else(|| Error::corrupt(path, "column length overflow"))?;
        if end_offset > body.len() { return Err(Error::corrupt(path, "column exceeds body")); }
    }
    if end_offset != body.len() { return Err(Error::corrupt(path, "trailing block body bytes")); }
    let decode_names: Vec<String> = match project {''')
