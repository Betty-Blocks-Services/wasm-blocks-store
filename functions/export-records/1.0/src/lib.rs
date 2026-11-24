mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::ExportRecords;
    export!(ExportRecords);
}

mod cell;
mod export;
mod filter;
mod format;
// A copy of `data-api-request`'s `graphql.rs`; this step does not use all of it.
#[allow(dead_code)]
mod graphql;
mod mapping;
mod template;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::bindings::betty_blocks_types::data_api::data_api::{HelperContext, request};
use crate::bindings::betty_blocks_types::types::types::{BettyModel, BettyProperty};
use crate::bindings::betty_blocks_types::upload_file::upload_file;
use crate::bindings::exports::betty_blocks::export_records::export_records::{
    Delimiter, FileType as WitFileType, Guest, JsonString, SortOrder,
};
use crate::export::{FileType, Params};

struct ExportRecords;

impl Guest for ExportRecords {
    #[allow(clippy::too_many_arguments)]
    fn export_records(
        helper_context: HelperContext,
        file_type: WitFileType,
        model_source: BettyModel,
        filter: Option<String>,
        filter_variables: JsonString,
        property_sort: Option<Vec<BettyProperty>>,
        sort_order: SortOrder,
        delimiter: Delimiter,
        model_target: BettyModel,
        property_target: Vec<BettyProperty>,
        file_name: Option<String>,
        export_property_mapping: JsonString,
        format_property_mapping: JsonString,
        use_bom: Option<bool>,
    ) -> Result<String, String> {
        let file_type = match file_type {
            WitFileType::Csv => FileType::Csv,
            WitFileType::Excel => FileType::Xlsx,
        };
        let sort_order = match sort_order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let delimiter = match delimiter {
            Delimiter::Semicolon => ";",
            Delimiter::Comma => ",",
            Delimiter::Pipe => "|",
            Delimiter::Tab => "\t",
            Delimiter::Space => " ",
        };
        let variables = parse_variables(&filter_variables)?;
        let mapping = mapping::parse_map(&export_property_mapping, "The Property mapping")?;
        let formats = mapping::parse_map(&format_property_mapping, "The Format mapping")?;
        let property_target = property_target
            .into_iter()
            .next()
            .ok_or("Select the property to save the file into")?;
        let sort_field = property_sort
            .as_ref()
            .and_then(|properties| properties.first())
            .map(|property| property.name.as_str());

        let params = Params {
            file_type,
            model: &model_source.name,
            filter: filter.as_deref().unwrap_or_default(),
            variables: &variables,
            sort_field,
            sort_order,
            delimiter,
            sheet: &model_target.name,
            mapping: &mapping,
            formats: &formats,
            bom: use_bom.unwrap_or(true),
        };

        let records = export::fetch_all(&params, |query| request(&helper_context, query, "{}"))?;
        let bytes = export::write_file(&params, &records)?;

        let result = upload_file::upload(
            &helper_context,
            &upload_file::Input {
                model: model_target,
                property: property_target,
                file_base64: BASE64.encode(bytes),
                full_filename: export::full_file_name(file_name.as_deref(), file_type),
            },
        )
        .map_err(|error| format!("Storing the file failed: {error}"))?;

        Ok(result.reference)
    }
}

/// The Variables in filter option as a JSON object. Empty (and `null`, which an unset Map may
/// arrive as) means no variables.
fn parse_variables(text: &str) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Null) => Ok(serde_json::json!({})),
        Ok(object @ serde_json::Value::Object(_)) => Ok(object),
        Ok(_) => Err(String::from(
            "The Variables in filter must be a map of names to values",
        )),
        Err(error) => Err(format!(
            "The Variables in filter are not valid JSON ({error})"
        )),
    }
}
