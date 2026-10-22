mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::DataApiRequest;
    export!(DataApiRequest);
}

mod graphql;
mod query;
mod template;

use crate::bindings::betty_blocks_types::data_api::data_api::{HelperContext, request};
use crate::bindings::betty_blocks_types::types::types::BettyModel;
use crate::bindings::exports::betty_blocks::data_api_request::data_api_request::{
    Guest, JsonString,
};
use crate::query::{OutputType, Params, QueryType, SortOrder};

struct DataApiRequest;

impl Guest for DataApiRequest {
    #[allow(clippy::too_many_arguments)]
    fn data_api_request(
        helper_context: HelperContext,
        model: BettyModel,
        query_type: String,
        filter: Option<String>,
        filter_variables: JsonString,
        skip: Option<u32>,
        take: Option<u32>,
        sort_order: String,
        sort_field: Option<String>,
        query_data: Option<String>,
        output_type: String,
        // Declared because the SchemaModel option in function.json is handed to the step by that
        // name; it only types the result on the canvas and has no effect on the query.
        _schema_model: Option<String>,
    ) -> Result<JsonString, String> {
        let variables: serde_json::Value = if filter_variables.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&filter_variables)
                .map_err(|error| format!("The variables are not valid JSON: {error}"))?
        };

        let params = Params {
            model_name: &model.name,
            query_type: QueryType::parse(&query_type)?,
            filter: filter.as_deref(),
            variables: &variables,
            skip,
            take,
            sort_order: SortOrder::parse(&sort_order)?,
            sort_field: sort_field.as_deref(),
            query_data: query_data.as_deref(),
            output_type: OutputType::parse(&output_type)?,
        };

        query::run(&params, |query| request(&helper_context, query, "{}"))
    }
}
