//! Builds the Data API query from the step's options and picks the requested value out of the
//! response. Pure functions, so everything here is testable without a host.

use serde_json::Value;

use crate::template;

/// Used when `take` is empty or 0, like the ActionsJS step.
pub const DEFAULT_TAKE: u32 = 200;
/// The ActionsJS step capped `take` at 5000. No Data API document states a real server-side
/// limit; this is checked on a canvas (see the plan's take-limit probe) before it is trusted.
pub const MAX_TAKE: u32 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    Record,
    Collection,
}

impl QueryType {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "record" => Ok(Self::Record),
            "collection" => Ok(Self::Collection),
            other => Err(format!(
                "Type must be \"record\" or \"collection\", got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    Results,
    TotalCount,
}

impl OutputType {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "" | "results" => Ok(Self::Results),
            "totalCount" => Ok(Self::TotalCount),
            other => Err(format!(
                "Output type must be \"results\" or \"totalCount\", got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "" | "ASC" => Ok(Self::Asc),
            "DESC" => Ok(Self::Desc),
            other => Err(format!(
                "Sort order must be \"ASC\" or \"DESC\", got \"{other}\""
            )),
        }
    }

    fn as_graphql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

pub struct Params<'a> {
    pub model_name: &'a str,
    pub query_type: QueryType,
    pub filter: Option<&'a str>,
    pub variables: &'a Value,
    pub skip: Option<u32>,
    pub take: Option<u32>,
    pub sort_order: SortOrder,
    pub sort_field: Option<&'a str>,
    pub query_data: Option<&'a str>,
    pub output_type: OutputType,
}

#[derive(Debug)]
pub struct BuiltQuery {
    pub query: String,
    /// The root field of the query (`oneWebuser`, `allWebuser`), where the data is in the response.
    pub field: String,
}

pub fn build_query(params: &Params) -> Result<BuiltQuery, String> {
    let model_name = params.model_name.trim();
    if model_name.is_empty() {
        return Err(String::from("Select a model"));
    }

    let field = match params.query_type {
        QueryType::Record => format!("one{model_name}"),
        QueryType::Collection => format!("all{model_name}"),
    };

    let query_data = params.query_data.map(str::trim).unwrap_or_default();
    let needs_query_data =
        params.query_type == QueryType::Record || params.output_type == OutputType::Results;
    if needs_query_data && query_data.is_empty() {
        return Err(String::from(
            "Query data is empty: list the properties to retrieve, e.g. \"id name\"",
        ));
    }

    let body = match (params.query_type, params.output_type) {
        (QueryType::Record, _) => query_data.to_string(),
        (QueryType::Collection, OutputType::Results) => format!("results {{ {query_data} }}"),
        (QueryType::Collection, OutputType::TotalCount) => String::from("totalCount"),
    };

    check_balanced("Query data", query_data)?;
    let filter = template::render(params.filter.unwrap_or_default(), params.variables)?;
    let filter = normalize_filter(&filter)?;
    let skip = params.skip.unwrap_or(0);
    let take = match params.take {
        None | Some(0) => DEFAULT_TAKE,
        Some(take) => take.min(MAX_TAKE),
    };
    let sort = match params
        .sort_field
        .map(str::trim)
        .filter(|field| !field.is_empty())
    {
        Some(sort_field) => format!(
            ", sort: {{ order: {}, field: {sort_field} }}",
            params.sort_order.as_graphql()
        ),
        None => String::new(),
    };

    let query = format!(
        "query {{ {field}(where: {{ {filter} }}, skip: {skip}, take: {take}{sort}) {{ {body} }} }}"
    );

    Ok(BuiltQuery { query, field })
}

/// How deep the brackets nest at each point of `text`, ignoring anything inside double-quoted
/// strings. Returns the depth at the end, or the offending closing character.
fn bracket_depths(text: &str) -> Result<Vec<(usize, i32)>, char> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut depths = Vec::new();

    for (index, character) in text.char_indices() {
        if in_string {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(character);
                }
            }
            _ => {}
        }
        depths.push((index, depth));
    }
    depths.push((text.len(), depth));
    Ok(depths)
}

/// Fails with a readable message when brackets do not pair up, instead of sending the query and
/// getting the Data API's "syntax error before: '}'".
fn check_balanced(label: &str, text: &str) -> Result<(), String> {
    match bracket_depths(text) {
        Err(closing) => Err(format!(
            "{label} has a closing `{closing}` without an opening one. \
             The filter is wrapped in curly braces for you, so write it without outer braces, \
             for example: id: {{ eq: 1 }}"
        )),
        Ok(depths) if depths.last().is_some_and(|(_, depth)| *depth > 0) => Err(format!(
            "{label} has an opening bracket that is never closed: check the `{{`, `[` and `(` pairs"
        )),
        Ok(_) => Ok(()),
    }
}

/// Checks the rendered filter and removes one pair of outer curly braces if the whole filter is
/// wrapped in them (`{ id: { eq: 1 } }`), because the step adds those itself. A filter can never
/// validly start with `{`, so this cannot change the meaning of a working filter.
fn normalize_filter(filter: &str) -> Result<String, String> {
    check_balanced("Filter", filter)?;
    let trimmed = filter.trim();

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let depths = bracket_depths(trimmed).unwrap_or_default();
        // The opening brace closes at the very end, not earlier (`{ a: 1 } { b: 2 }`).
        let closes_only_at_end = depths
            .iter()
            .take(depths.len().saturating_sub(2))
            .all(|(_, depth)| *depth > 0);
        if closes_only_at_end {
            return Ok(trimmed[1..trimmed.len() - 1].trim().to_string());
        }
    }
    Ok(trimmed.to_string())
}

/// The messages of a GraphQL error envelope (`{"errors": [{"message": ...}]}`), if it is one.
fn graphql_error_messages(response: &Value) -> Option<String> {
    let errors = response.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let messages: Vec<String> = errors
        .iter()
        .map(|error| match error.get("message").and_then(Value::as_str) {
            Some(message) => message.to_string(),
            None => error.to_string(),
        })
        .collect();
    Some(messages.join("; "))
}

/// The host reports a backend error as `Err(<the GraphQL error envelope as a JSON string>)`.
/// Turn that into the plain message when it is one, otherwise keep the text as it is.
pub fn host_error_message(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|parsed| graphql_error_messages(&parsed))
        .unwrap_or_else(|| raw.to_string())
}

/// Picks the value the step returns out of the Data API response, as JSON text.
pub fn extract_result(
    response: &str,
    built: &BuiltQuery,
    params: &Params,
) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(response)
        .map_err(|error| format!("The Data API response is not valid JSON: {error}"))?;

    if let Some(messages) = graphql_error_messages(&parsed) {
        return Err(messages);
    }

    let node = parsed
        .get("data")
        .and_then(|data| data.get(&built.field))
        .ok_or_else(|| format!("The Data API response has no data for `{}`", built.field))?;

    let value = match (params.query_type, params.output_type) {
        (QueryType::Record, _) => node,
        (QueryType::Collection, OutputType::Results) => node
            .get("results")
            .ok_or("The Data API response has no `results`")?,
        (QueryType::Collection, OutputType::TotalCount) => node
            .get("totalCount")
            .ok_or("The Data API response has no `totalCount`")?,
    };

    serde_json::to_string(value).map_err(|error| error.to_string())
}

/// Builds the query, sends it with `send`, and returns the requested value as JSON text.
pub fn run(
    params: &Params,
    send: impl FnOnce(&str) -> Result<String, String>,
) -> Result<String, String> {
    let built = build_query(params)?;
    let response = send(&built.query).map_err(|error| host_error_message(&error))?;
    extract_result(&response, &built, params)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn params(variables: &Value) -> Params<'_> {
        Params {
            model_name: "Webuser",
            query_type: QueryType::Collection,
            filter: None,
            variables,
            skip: None,
            take: None,
            sort_order: SortOrder::Asc,
            sort_field: None,
            query_data: Some("id name"),
            output_type: OutputType::Results,
        }
    }

    fn squash(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn collection_query_with_defaults() {
        let vars = json!({});
        let built = build_query(&params(&vars)).unwrap();
        assert_eq!(built.field, "allWebuser");
        assert_eq!(
            squash(&built.query),
            "query { allWebuser(where: { }, skip: 0, take: 200) { results { id name } } }"
        );
    }

    #[test]
    fn record_query_uses_the_query_data_directly() {
        let vars = json!({ "id": "1" });
        let mut p = params(&vars);
        p.query_type = QueryType::Record;
        p.filter = Some("id: { eq: {{id}} }");
        let built = build_query(&p).unwrap();
        assert_eq!(built.field, "oneWebuser");
        assert_eq!(
            squash(&built.query),
            "query { oneWebuser(where: { id: { eq: 1 } }, skip: 0, take: 200) { id name } }"
        );
    }

    #[test]
    fn total_count_needs_no_query_data() {
        let vars = json!({});
        let mut p = params(&vars);
        p.output_type = OutputType::TotalCount;
        p.query_data = None;
        assert_eq!(
            squash(&build_query(&p).unwrap().query),
            "query { allWebuser(where: { }, skip: 0, take: 200) { totalCount } }"
        );
    }

    #[test]
    fn sort_is_added_only_when_a_field_is_given() {
        let vars = json!({});
        let mut p = params(&vars);
        p.sort_order = SortOrder::Desc;
        p.sort_field = Some("name");
        assert!(
            squash(&build_query(&p).unwrap().query)
                .contains("take: 200, sort: { order: DESC, field: name })")
        );
        p.sort_field = Some("  ");
        assert!(!build_query(&p).unwrap().query.contains("sort"));
    }

    #[test]
    fn take_defaults_to_200_and_is_capped() {
        let vars = json!({});
        let mut p = params(&vars);
        for (take, expected) in [
            (None, 200),
            (Some(0), 200),
            (Some(50), 50),
            (Some(9999), 5000),
        ] {
            p.take = take;
            assert!(
                build_query(&p)
                    .unwrap()
                    .query
                    .contains(&format!("take: {expected}")),
                "take {take:?}"
            );
        }
        p.skip = Some(30);
        assert!(build_query(&p).unwrap().query.contains("skip: 30"));
    }

    #[test]
    fn empty_query_data_and_model_are_reported() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("  ");
        assert!(build_query(&p).unwrap_err().contains("Query data is empty"));
        p.query_data = Some("id");
        p.model_name = " ";
        assert!(build_query(&p).unwrap_err().contains("Select a model"));
    }

    #[test]
    fn template_errors_are_returned() {
        let vars = json!({});
        let mut p = params(&vars);
        p.filter = Some("{{#a}}open");
        assert!(build_query(&p).unwrap_err().contains("never closed"));
    }

    #[test]
    fn option_values_are_validated() {
        assert!(QueryType::parse("record").is_ok());
        assert!(QueryType::parse("both").is_err());
        assert_eq!(OutputType::parse("").unwrap(), OutputType::Results);
        assert!(OutputType::parse("count").is_err());
        assert_eq!(SortOrder::parse("").unwrap(), SortOrder::Asc);
        assert!(SortOrder::parse("up").is_err());
    }

    fn run_with(response: &str, p: &Params) -> Result<String, String> {
        run(p, |_| Ok(response.to_string()))
    }

    #[test]
    fn extracts_results_total_count_and_records() {
        let vars = json!({});
        let mut p = params(&vars);
        assert_eq!(
            run_with(
                r#"{"data":{"allWebuser":{"results":[{"id":1}],"totalCount":9}}}"#,
                &p
            )
            .unwrap(),
            r#"[{"id":1}]"#
        );

        p.output_type = OutputType::TotalCount;
        assert_eq!(
            run_with(r#"{"data":{"allWebuser":{"totalCount":9}}}"#, &p).unwrap(),
            "9"
        );

        p.query_type = QueryType::Record;
        assert_eq!(
            run_with(r#"{"data":{"oneWebuser":{"id":1,"name":"M"}}}"#, &p).unwrap(),
            r#"{"id":1,"name":"M"}"#
        );
        assert_eq!(
            run_with(r#"{"data":{"oneWebuser":null}}"#, &p).unwrap(),
            "null"
        );
    }

    #[test]
    fn graphql_errors_in_a_response_fail_the_step() {
        let vars = json!({});
        let p = params(&vars);
        let error = run_with(
            r#"{"errors":[{"message":"first"},{"message":"second"}],"data":null}"#,
            &p,
        )
        .unwrap_err();
        assert_eq!(error, "first; second");
    }

    #[test]
    fn host_errors_are_unwrapped_to_the_message() {
        let vars = json!({});
        let p = params(&vars);
        let envelope = r#"{"errors":[{"extensions":{"statusCode":400},"message":"in object 'Webuser' the fields 'ida' are not found."}]}"#;
        let error = run(&p, |_| Err(envelope.to_string())).unwrap_err();
        assert_eq!(error, "in object 'Webuser' the fields 'ida' are not found.");

        // Anything that is not a GraphQL envelope is kept as it is.
        let error = run(&p, |_| Err(String::from("gRPC call failed"))).unwrap_err();
        assert_eq!(error, "gRPC call failed");
    }

    #[test]
    fn responses_without_the_expected_data_are_reported() {
        let vars = json!({});
        let p = params(&vars);
        assert!(
            run_with("not json", &p)
                .unwrap_err()
                .contains("not valid JSON")
        );
        assert!(
            run_with(r#"{"data":{}}"#, &p)
                .unwrap_err()
                .contains("allWebuser")
        );
        assert!(
            run_with(r#"{"data":{"allWebuser":{}}}"#, &p)
                .unwrap_err()
                .contains("results")
        );
    }

    #[test]
    fn the_query_is_not_sent_when_it_cannot_be_built() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = None;
        let result = run(&p, |_| panic!("must not be sent"));
        assert!(result.is_err());
    }

    fn filter_clause(filter: &str) -> Result<String, String> {
        let vars = json!({});
        let mut p = params(&vars);
        p.filter = Some(filter);
        build_query(&p).map(|built| squash(&built.query))
    }

    #[test]
    fn an_extra_closing_brace_gets_a_readable_error() {
        let error = filter_clause("id: { eq: 1 } }").unwrap_err();
        assert!(
            error.contains("closing `}` without an opening one"),
            "{error}"
        );
        assert!(error.contains("without outer braces"), "{error}");
    }

    #[test]
    fn a_missing_closing_brace_gets_a_readable_error() {
        let error = filter_clause("id: { eq: 1 ").unwrap_err();
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn a_filter_wrapped_in_braces_is_unwrapped() {
        let expected = "query { allWebuser(where: { id: { eq: 1 } }, skip: 0, take: 200) { results { id name } } }";
        assert_eq!(filter_clause("{ id: { eq: 1 } }").unwrap(), expected);
        assert_eq!(filter_clause("id: { eq: 1 }").unwrap(), expected);
        assert_eq!(
            filter_clause("  {id: {eq: 1}}  ").unwrap(),
            expected.replace("{ id: { eq: 1 } }", "{ id: {eq: 1} }")
        );
    }

    #[test]
    fn two_separate_objects_are_not_mistaken_for_one_wrapped_filter() {
        // Not valid GraphQL either way, but it must reach the Data API unchanged, not be mangled.
        let query = filter_clause("{ a: 1 } { b: 2 }").unwrap();
        assert!(query.contains("where: { { a: 1 } { b: 2 } }"), "{query}");
    }

    #[test]
    fn braces_inside_text_values_do_not_count() {
        let query = filter_clause("name: { eq: \"a } b \\\" {\" }").unwrap();
        assert!(query.contains("name: { eq: \"a } b \\\" {\" }"), "{query}");
    }

    #[test]
    fn unbalanced_query_data_is_reported() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("id company { id");
        assert!(build_query(&p).unwrap_err().contains("Query data"));
    }

    /// The query the original ActionsJS step 1.7 builds (see tests/generate_query_cases.mjs).
    #[test]
    fn builds_the_same_queries_as_the_original_actionsjs_step() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/data_api_request_cases.json")).unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let text = |key: &str| case[key].as_str();
            let variables = &case["variables"];
            let params = Params {
                model_name: text("model").unwrap(),
                query_type: QueryType::parse(text("queryType").unwrap()).unwrap(),
                filter: text("filter"),
                variables,
                skip: case["skip"].as_u64().map(|n| n as u32),
                take: case["take"].as_u64().map(|n| n as u32),
                sort_order: SortOrder::parse(text("sortOrder").unwrap()).unwrap(),
                sort_field: text("sortField"),
                query_data: text("queryData"),
                output_type: OutputType::parse(text("outputType").unwrap()).unwrap(),
            };
            assert_eq!(
                squash(&build_query(&params).unwrap().query),
                case["expectedQuery"].as_str().unwrap(),
                "case {case}"
            );
        }
    }
}
