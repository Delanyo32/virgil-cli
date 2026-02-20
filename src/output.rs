use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{BooleanArray, StringArray, UInt32Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;

use crate::models::{CommentInfo, FileMetadata, ImportInfo, ParseError, SymbolInfo};

fn files_schema() -> Schema {
    Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("extension", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("size_bytes", DataType::UInt64, false),
        Field::new("line_count", DataType::UInt64, false),
    ])
}

fn symbols_schema() -> Schema {
    Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("file_path", DataType::Utf8, false),
        Field::new("start_line", DataType::UInt32, false),
        Field::new("start_column", DataType::UInt32, false),
        Field::new("end_line", DataType::UInt32, false),
        Field::new("end_column", DataType::UInt32, false),
        Field::new("is_exported", DataType::Boolean, false),
    ])
}

pub fn write_files_parquet(files: &[FileMetadata], output_dir: &Path) -> Result<()> {
    let schema = Arc::new(files_schema());

    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    let extensions: Vec<&str> = files.iter().map(|f| f.extension.as_str()).collect();
    let languages: Vec<&str> = files.iter().map(|f| f.language.as_str()).collect();
    let sizes: Vec<u64> = files.iter().map(|f| f.size_bytes).collect();
    let lines: Vec<u64> = files.iter().map(|f| f.line_count).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(paths)),
            Arc::new(StringArray::from(names)),
            Arc::new(StringArray::from(extensions)),
            Arc::new(StringArray::from(languages)),
            Arc::new(UInt64Array::from(sizes)),
            Arc::new(UInt64Array::from(lines)),
        ],
    )
    .context("failed to create files RecordBatch")?;

    let path = output_dir.join("files.parquet");
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer =
        ArrowWriter::try_new(file, schema, None).context("failed to create parquet writer")?;
    writer
        .write(&batch)
        .context("failed to write files batch")?;
    writer.close().context("failed to close parquet writer")?;

    Ok(())
}

pub fn write_symbols_parquet(symbols: &[SymbolInfo], output_dir: &Path) -> Result<()> {
    let schema = Arc::new(symbols_schema());

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    let kinds: Vec<String> = symbols.iter().map(|s| s.kind.to_string()).collect();
    let kind_refs: Vec<&str> = kinds.iter().map(|s| s.as_str()).collect();
    let file_paths: Vec<&str> = symbols.iter().map(|s| s.file_path.as_str()).collect();
    let start_lines: Vec<u32> = symbols.iter().map(|s| s.start_line).collect();
    let start_cols: Vec<u32> = symbols.iter().map(|s| s.start_column).collect();
    let end_lines: Vec<u32> = symbols.iter().map(|s| s.end_line).collect();
    let end_cols: Vec<u32> = symbols.iter().map(|s| s.end_column).collect();
    let exported: Vec<bool> = symbols.iter().map(|s| s.is_exported).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(names)),
            Arc::new(StringArray::from(kind_refs)),
            Arc::new(StringArray::from(file_paths)),
            Arc::new(UInt32Array::from(start_lines)),
            Arc::new(UInt32Array::from(start_cols)),
            Arc::new(UInt32Array::from(end_lines)),
            Arc::new(UInt32Array::from(end_cols)),
            Arc::new(BooleanArray::from(exported)),
        ],
    )
    .context("failed to create symbols RecordBatch")?;

    let path = output_dir.join("symbols.parquet");
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer =
        ArrowWriter::try_new(file, schema, None).context("failed to create parquet writer")?;
    writer
        .write(&batch)
        .context("failed to write symbols batch")?;
    writer.close().context("failed to close parquet writer")?;

    Ok(())
}

fn imports_schema() -> Schema {
    Schema::new(vec![
        Field::new("source_file", DataType::Utf8, false),
        Field::new("module_specifier", DataType::Utf8, false),
        Field::new("imported_name", DataType::Utf8, false),
        Field::new("local_name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("is_type_only", DataType::Boolean, false),
        Field::new("line", DataType::UInt32, false),
        Field::new("is_external", DataType::Boolean, false),
    ])
}

pub fn write_imports_parquet(imports: &[ImportInfo], output_dir: &Path) -> Result<()> {
    let schema = Arc::new(imports_schema());

    let source_files: Vec<&str> = imports.iter().map(|i| i.source_file.as_str()).collect();
    let module_specifiers: Vec<&str> = imports
        .iter()
        .map(|i| i.module_specifier.as_str())
        .collect();
    let imported_names: Vec<&str> = imports.iter().map(|i| i.imported_name.as_str()).collect();
    let local_names: Vec<&str> = imports.iter().map(|i| i.local_name.as_str()).collect();
    let kinds: Vec<&str> = imports.iter().map(|i| i.kind.as_str()).collect();
    let is_type_only: Vec<bool> = imports.iter().map(|i| i.is_type_only).collect();
    let lines: Vec<u32> = imports.iter().map(|i| i.line).collect();
    let is_external: Vec<bool> = imports.iter().map(|i| i.is_external).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(source_files)),
            Arc::new(StringArray::from(module_specifiers)),
            Arc::new(StringArray::from(imported_names)),
            Arc::new(StringArray::from(local_names)),
            Arc::new(StringArray::from(kinds)),
            Arc::new(BooleanArray::from(is_type_only)),
            Arc::new(UInt32Array::from(lines)),
            Arc::new(BooleanArray::from(is_external)),
        ],
    )
    .context("failed to create imports RecordBatch")?;

    let path = output_dir.join("imports.parquet");
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer =
        ArrowWriter::try_new(file, schema, None).context("failed to create parquet writer")?;
    writer
        .write(&batch)
        .context("failed to write imports batch")?;
    writer.close().context("failed to close parquet writer")?;

    Ok(())
}

fn comments_schema() -> Schema {
    Schema::new(vec![
        Field::new("file_path", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("start_line", DataType::UInt32, false),
        Field::new("start_column", DataType::UInt32, false),
        Field::new("end_line", DataType::UInt32, false),
        Field::new("end_column", DataType::UInt32, false),
        Field::new("associated_symbol", DataType::Utf8, true),
        Field::new("associated_symbol_kind", DataType::Utf8, true),
    ])
}

pub fn write_comments_parquet(comments: &[CommentInfo], output_dir: &Path) -> Result<()> {
    let schema = Arc::new(comments_schema());

    let file_paths: Vec<&str> = comments.iter().map(|c| c.file_path.as_str()).collect();
    let texts: Vec<&str> = comments.iter().map(|c| c.text.as_str()).collect();
    let kinds: Vec<&str> = comments.iter().map(|c| c.kind.as_str()).collect();
    let start_lines: Vec<u32> = comments.iter().map(|c| c.start_line).collect();
    let start_cols: Vec<u32> = comments.iter().map(|c| c.start_column).collect();
    let end_lines: Vec<u32> = comments.iter().map(|c| c.end_line).collect();
    let end_cols: Vec<u32> = comments.iter().map(|c| c.end_column).collect();
    let associated_symbols: Vec<Option<&str>> = comments
        .iter()
        .map(|c| c.associated_symbol.as_deref())
        .collect();
    let associated_symbol_kinds: Vec<Option<&str>> = comments
        .iter()
        .map(|c| c.associated_symbol_kind.as_deref())
        .collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(file_paths)),
            Arc::new(StringArray::from(texts)),
            Arc::new(StringArray::from(kinds)),
            Arc::new(UInt32Array::from(start_lines)),
            Arc::new(UInt32Array::from(start_cols)),
            Arc::new(UInt32Array::from(end_lines)),
            Arc::new(UInt32Array::from(end_cols)),
            Arc::new(StringArray::from(associated_symbols)),
            Arc::new(StringArray::from(associated_symbol_kinds)),
        ],
    )
    .context("failed to create comments RecordBatch")?;

    let path = output_dir.join("comments.parquet");
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer =
        ArrowWriter::try_new(file, schema, None).context("failed to create parquet writer")?;
    writer
        .write(&batch)
        .context("failed to write comments batch")?;
    writer.close().context("failed to close parquet writer")?;

    Ok(())
}

fn errors_schema() -> Schema {
    Schema::new(vec![
        Field::new("file_path", DataType::Utf8, false),
        Field::new("file_name", DataType::Utf8, false),
        Field::new("extension", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("error_type", DataType::Utf8, false),
        Field::new("error_message", DataType::Utf8, false),
        Field::new("size_bytes", DataType::UInt64, false),
    ])
}

pub fn write_errors_parquet(errors: &[ParseError], output_dir: &Path) -> Result<()> {
    let schema = Arc::new(errors_schema());

    let file_paths: Vec<&str> = errors.iter().map(|e| e.file_path.as_str()).collect();
    let file_names: Vec<&str> = errors.iter().map(|e| e.file_name.as_str()).collect();
    let extensions: Vec<&str> = errors.iter().map(|e| e.extension.as_str()).collect();
    let languages: Vec<&str> = errors.iter().map(|e| e.language.as_str()).collect();
    let error_types: Vec<&str> = errors.iter().map(|e| e.error_type.as_str()).collect();
    let error_messages: Vec<&str> = errors.iter().map(|e| e.error_message.as_str()).collect();
    let sizes: Vec<u64> = errors.iter().map(|e| e.size_bytes).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(file_paths)),
            Arc::new(StringArray::from(file_names)),
            Arc::new(StringArray::from(extensions)),
            Arc::new(StringArray::from(languages)),
            Arc::new(StringArray::from(error_types)),
            Arc::new(StringArray::from(error_messages)),
            Arc::new(UInt64Array::from(sizes)),
        ],
    )
    .context("failed to create errors RecordBatch")?;

    let path = output_dir.join("errors.parquet");
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer =
        ArrowWriter::try_new(file, schema, None).context("failed to create parquet writer")?;
    writer
        .write(&batch)
        .context("failed to write errors batch")?;
    writer.close().context("failed to close parquet writer")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::AsArray;
    use arrow::datatypes::UInt32Type;
    use arrow::datatypes::UInt64Type;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    #[test]
    fn files_schema_has_six_columns() {
        let schema = files_schema();
        assert_eq!(schema.fields().len(), 6);
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "path",
                "name",
                "extension",
                "language",
                "size_bytes",
                "line_count"
            ]
        );
    }

    #[test]
    fn symbols_schema_has_eight_columns() {
        let schema = symbols_schema();
        assert_eq!(schema.fields().len(), 8);
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "name",
                "kind",
                "file_path",
                "start_line",
                "start_column",
                "end_line",
                "end_column",
                "is_exported"
            ]
        );
    }

    #[test]
    fn write_files_parquet_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            FileMetadata {
                path: "src/main.ts".to_string(),
                name: "main.ts".to_string(),
                extension: "ts".to_string(),
                language: "typescript".to_string(),
                size_bytes: 1024,
                line_count: 50,
            },
            FileMetadata {
                path: "src/util.js".to_string(),
                name: "util.js".to_string(),
                extension: "js".to_string(),
                language: "javascript".to_string(),
                size_bytes: 512,
                line_count: 20,
            },
        ];

        write_files_parquet(&files, dir.path()).expect("write");

        let path = dir.path().join("files.parquet");
        let file = File::open(&path).expect("open");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("reader builder")
            .build()
            .expect("reader");

        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);

        let paths = batch.column(0).as_string::<i32>();
        assert_eq!(paths.value(0), "src/main.ts");
        assert_eq!(paths.value(1), "src/util.js");

        let names = batch.column(1).as_string::<i32>();
        assert_eq!(names.value(0), "main.ts");

        let sizes = batch.column(4).as_primitive::<UInt64Type>();
        assert_eq!(sizes.value(0), 1024);
        assert_eq!(sizes.value(1), 512);

        let lines = batch.column(5).as_primitive::<UInt64Type>();
        assert_eq!(lines.value(0), 50);
    }

    #[test]
    fn write_symbols_parquet_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let symbols = vec![
            SymbolInfo {
                name: "greet".to_string(),
                kind: crate::models::SymbolKind::Function,
                file_path: "main.ts".to_string(),
                start_line: 0,
                start_column: 0,
                end_line: 2,
                end_column: 1,
                is_exported: true,
            },
            SymbolInfo {
                name: "PI".to_string(),
                kind: crate::models::SymbolKind::Variable,
                file_path: "main.ts".to_string(),
                start_line: 4,
                start_column: 0,
                end_line: 4,
                end_column: 20,
                is_exported: false,
            },
        ];

        write_symbols_parquet(&symbols, dir.path()).expect("write");

        let path = dir.path().join("symbols.parquet");
        let file = File::open(&path).expect("open");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("reader builder")
            .build()
            .expect("reader");

        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);

        let names = batch.column(0).as_string::<i32>();
        assert_eq!(names.value(0), "greet");
        assert_eq!(names.value(1), "PI");

        let kinds = batch.column(1).as_string::<i32>();
        assert_eq!(kinds.value(0), "function");
        assert_eq!(kinds.value(1), "variable");

        let start_lines = batch.column(3).as_primitive::<UInt32Type>();
        assert_eq!(start_lines.value(0), 0);
        assert_eq!(start_lines.value(1), 4);

        let exported = batch.column(7).as_boolean();
        assert!(exported.value(0));
        assert!(!exported.value(1));
    }

    #[test]
    fn write_empty_files_parquet() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_files_parquet(&[], dir.path()).expect("write empty");
        let path = dir.path().join("files.parquet");
        assert!(path.exists());

        let file = File::open(&path).expect("open");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("reader builder")
            .build()
            .expect("reader");
        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read");
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 0);
    }

    #[test]
    fn write_empty_symbols_parquet() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_symbols_parquet(&[], dir.path()).expect("write empty");
        let path = dir.path().join("symbols.parquet");
        assert!(path.exists());
    }

    #[test]
    fn imports_schema_has_eight_columns() {
        let schema = imports_schema();
        assert_eq!(schema.fields().len(), 8);
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "source_file",
                "module_specifier",
                "imported_name",
                "local_name",
                "kind",
                "is_type_only",
                "line",
                "is_external"
            ]
        );
    }

    #[test]
    fn write_imports_parquet_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let imports = vec![
            ImportInfo {
                source_file: "src/main.ts".to_string(),
                module_specifier: "./utils".to_string(),
                imported_name: "foo".to_string(),
                local_name: "foo".to_string(),
                kind: "static".to_string(),
                is_type_only: false,
                line: 0,
                is_external: false,
            },
            ImportInfo {
                source_file: "src/main.ts".to_string(),
                module_specifier: "react".to_string(),
                imported_name: "default".to_string(),
                local_name: "React".to_string(),
                kind: "static".to_string(),
                is_type_only: false,
                line: 1,
                is_external: true,
            },
        ];

        write_imports_parquet(&imports, dir.path()).expect("write");

        let path = dir.path().join("imports.parquet");
        let file = File::open(&path).expect("open");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("reader builder")
            .build()
            .expect("reader");

        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);

        let source_files = batch.column(0).as_string::<i32>();
        assert_eq!(source_files.value(0), "src/main.ts");

        let module_specs = batch.column(1).as_string::<i32>();
        assert_eq!(module_specs.value(0), "./utils");
        assert_eq!(module_specs.value(1), "react");

        let imported_names = batch.column(2).as_string::<i32>();
        assert_eq!(imported_names.value(0), "foo");
        assert_eq!(imported_names.value(1), "default");

        let local_names = batch.column(3).as_string::<i32>();
        assert_eq!(local_names.value(1), "React");

        let lines = batch.column(6).as_primitive::<UInt32Type>();
        assert_eq!(lines.value(0), 0);
        assert_eq!(lines.value(1), 1);

        let is_external = batch.column(7).as_boolean();
        assert!(!is_external.value(0)); // ./utils = internal
        assert!(is_external.value(1)); // react = external
    }

    #[test]
    fn write_empty_imports_parquet() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_imports_parquet(&[], dir.path()).expect("write empty");
        let path = dir.path().join("imports.parquet");
        assert!(path.exists());
    }
}
