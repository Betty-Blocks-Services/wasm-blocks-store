mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::DataApiQuery;
    export!(DataApiQuery);
}

// A copy of `data-api-request`'s `graphql.rs`; this step does not use all of it.
#[allow(dead_code)]
mod graphql;
mod query;
mod template;

use crate::bindings::betty_blocks_types::data_api::data_api::{HelperContext, request};
use crate::bindings::exports::betty_blocks::data_api_query::data_api_query::{Guest, JsonString};

struct DataApiQuery;

impl Guest for DataApiQuery {
    fn data_api_query(
        helper_context: HelperContext,
        query: String,
        variables: JsonString,
        _schema_model: Option<String>,
    ) -> Result<JsonString, String> {
        let variables: serde_json::Value = if variables.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&variables)
                .map_err(|error| format!("The variables are not valid JSON: {error}"))?
        };

        query::run(&query, &variables, |query| {
            request(&helper_context, query, "{}")
        })
    }
}
