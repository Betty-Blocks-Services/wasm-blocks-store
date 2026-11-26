// `waki` links against the official `wasi` crate's own generated bindings for `wasi:http`, so
// these bindings reuse those types (via `with:`) instead of generating a second, incompatible copy
// (the pattern of exchange-code-for-tokens and the native HTTP step).
mod bindings {
    wit_bindgen::generate!({
        world: "import-records-world",
        with: {
            "wasi:io/streams@0.2.6": ::wasi::io::streams,
            "wasi:io/error@0.2.6": ::wasi::io::error,
            "wasi:clocks/monotonic-clock@0.2.6": ::wasi::clocks::monotonic_clock,
            "wasi:io/poll@0.2.6": ::wasi::io::poll,
            "wasi:http/types@0.2.6": ::wasi::http::types,
            "wasi:http/outgoing-handler@0.2.6": ::wasi::http::outgoing_handler,
        },
        generate_all,
    });

    use crate::ImportRecords;
    export!(ImportRecords);
}

mod download;
mod format;
mod import;
mod mapping;
mod read;

use crate::bindings::betty_blocks_types::data_api::data_api::{HelperContext, request};
use crate::bindings::betty_blocks_types::types::types::BettyModel;
use crate::bindings::exports::betty_blocks::import_records::import_records::{
    Delimiter, FileType, Guest, JsonString, UniqueType as WitUniqueType,
};
use crate::import::{Options, Params, UniqueType};

struct ImportRecords;

impl Guest for ImportRecords {
    #[allow(clippy::too_many_arguments)]
    fn import_records(
        helper_context: HelperContext,
        file_url: String,
        file_type: FileType,
        delimiter: Delimiter,
        model: BettyModel,
        import_mapping: JsonString,
        format_mapping: JsonString,
        default_mapping: JsonString,
        deduplicate: Option<bool>,
        unique_column: Option<String>,
        unique_type: WitUniqueType,
        update_mapping: JsonString,
        validate_required_columns: Option<bool>,
    ) -> Result<String, String> {
        let url = download::check_url(&file_url)?;
        let params = Params::new(Options {
            model: &model.name,
            import_mapping: &import_mapping,
            format_mapping: &format_mapping,
            default_mapping: &default_mapping,
            deduplicate: deduplicate.unwrap_or(false),
            unique_column: unique_column.as_deref(),
            unique_type: match unique_type {
                WitUniqueType::Text => UniqueType::Text,
                WitUniqueType::Number => UniqueType::Number,
                WitUniqueType::Decimal => UniqueType::Decimal,
                WitUniqueType::Id => UniqueType::Id,
            },
            update_mapping: &update_mapping,
            validate_required: validate_required_columns.unwrap_or(false),
        })?;

        let bytes = download::download(url)?;
        let table = match file_type {
            FileType::Csv => read::read_csv(
                &bytes,
                match delimiter {
                    Delimiter::Auto => None,
                    Delimiter::Semicolon => Some(b';'),
                    Delimiter::Comma => Some(b','),
                    Delimiter::Pipe => Some(b'|'),
                    Delimiter::Tab => Some(b'\t'),
                },
            )?,
            FileType::Excel => read::read_excel(bytes)?,
        };

        let summary = import::run(&table, &params, &mut |query, variables| {
            request(&helper_context, query, variables)
        })?;
        Ok(summary.text())
    }
}
