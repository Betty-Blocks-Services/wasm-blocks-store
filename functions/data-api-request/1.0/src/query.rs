//! Builds the Data API query from the step's options and picks the requested value out of the
//! response. Pure functions, so everything here is testable without a host.

use serde_json::Value;

use crate::graphql::{self, Problem};
use crate::template;

/// Used when `take` is empty or 0, like the ActionsJS step.
pub const DEFAULT_TAKE: u32 = 200;
/// The most records the Data API returns per request: a `take` above this is rejected as an
/// unsupported request.
pub const PAGE_SIZE: u32 = 200;
/// The most records one step run returns, as in the ActionsJS step. A `take` above `PAGE_SIZE` is
/// fetched in several requests of `PAGE_SIZE` using `skip`.
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

#[derive(Debug, Clone, Copy)]
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

/// How many records the step returns at most: `take`, 200 when empty or 0, capped at `MAX_TAKE`.
pub fn requested_take(params: &Params) -> u32 {
    match params.take {
        None | Some(0) => DEFAULT_TAKE,
        Some(take) => take.min(MAX_TAKE),
    }
}

/// The query for the first (or only) request: at most `PAGE_SIZE` records.
pub fn build_query(params: &Params) -> Result<BuiltQuery, String> {
    build_page(
        params,
        params.skip.unwrap_or(0),
        requested_take(params).min(PAGE_SIZE),
    )
}

/// The query for one request that skips `skip` records and takes `take` (at most `PAGE_SIZE`).
fn build_page(params: &Params, skip: u32, take: u32) -> Result<BuiltQuery, String> {
    let model_name = params.model_name.trim();
    if model_name.is_empty() {
        return Err(String::from("Select a model"));
    }
    if !is_name(model_name, false) {
        return Err(format!(
            "The model name \"{model_name}\" is not valid: use letters, digits and underscores"
        ));
    }

    let field = match params.query_type {
        QueryType::Record => format!("one{model_name}"),
        QueryType::Collection => format!("all{model_name}"),
    };

    let needs_query_data =
        params.query_type == QueryType::Record || params.output_type == OutputType::Results;
    let query_data = if needs_query_data {
        let raw = params.query_data.unwrap_or_default();
        if raw.trim().is_empty() {
            return Err(String::from(
                "Query data is empty: list the properties to retrieve, e.g. \"id name\"",
            ));
        }
        // Checked as typed (not trimmed) so the line numbers in the message are the real ones.
        check_balanced("Query data", raw, true, "")?;
        let selection = raw.trim();
        // Later steps (an update, a loop, ...) find a record by its id, so the id is always selected.
        if graphql::selects_id(selection) {
            selection.to_string()
        } else {
            format!("id {selection}")
        }
    } else {
        String::new()
    };

    let body = match (params.query_type, params.output_type) {
        (QueryType::Record, _) => query_data,
        (QueryType::Collection, OutputType::Results) => format!("results {{ {query_data}\n}}"),
        (QueryType::Collection, OutputType::TotalCount) => String::from("totalCount"),
    };

    let template = params.filter.unwrap_or_default();
    let filter = template::render(template, params.variables)?;
    let filter = normalize_filter(&filter, template.contains("{{"))?;
    let sort = match params
        .sort_field
        .map(str::trim)
        .filter(|field| !field.is_empty())
    {
        Some(sort_field) if !is_name(sort_field, true) => {
            return Err(format!(
                "The sort field \"{sort_field}\" is not valid: use a property name, with a . for a relation (for example company.name)"
            ));
        }
        Some(sort_field) => format!(
            ", sort: {{ order: {}, field: {sort_field} }}",
            params.sort_order.as_graphql()
        ),
        None => String::new(),
    };

    let query = format!(
        "query {{ {field}(where: {{ {filter}\n}}, skip: {skip}, take: {take}{sort}) {{ {body}\n}} }}"
    );

    Ok(BuiltQuery { query, field })
}

/// Fails with a readable message that points at the line when the text has a string that is not
/// closed or brackets that do not pair up, instead of sending the query and getting the Data
/// API's "syntax error before: '}'".
///
/// `show_text` quotes the offending line; it is off for the filter, which can contain values
/// filled in from variables that should not end up in an error message.
fn check_balanced(label: &str, text: &str, show_text: bool, note: &str) -> Result<(), String> {
    match graphql::find_problem(text) {
        None => Ok(()),
        Some(problem) => {
            let hint = if matches!(problem, Problem::UnmatchedClose { .. }) {
                " The filter is wrapped in curly braces for you, so write it without outer \
                 braces, for example: id: { eq: 1 }."
            } else {
                ""
            };
            let hint = if label == "Filter" { hint } else { "" };
            Err(format!(
                "{label} {}{note}.{hint}",
                problem.describe(text, show_text)
            ))
        }
    }
}

/// Checks the rendered filter and removes one pair of outer curly braces if the whole filter is
/// wrapped in them (`{ id: { eq: 1 } }`), because the step adds those itself. A filter can never
/// validly start with `{`, so this cannot change the meaning of a working filter.
///
/// Line numbers refer to the rendered text, so when the filter template has `{{...}}` tags the
/// message says so.
fn normalize_filter(filter: &str, was_templated: bool) -> Result<String, String> {
    let note = if was_templated {
        " (counted after the variables were filled in)"
    } else {
        ""
    };
    check_balanced("Filter", filter, false, note)?;
    let trimmed = filter.trim();
    Ok(graphql::inside_outer_braces(trimmed)
        .unwrap_or(trimmed)
        .to_string())
}

/// A property or model name: letters, digits and `_`, and `.` too when `allow_dot` is set (for a
/// relation path in a sort field).
fn is_name(text: &str, allow_dot: bool) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_' || (allow_dot && c == '.'))
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
        (QueryType::Collection, OutputType::Results) => match node.get("results") {
            // `null` is an empty list, whether the step fetched one page or several.
            Some(Value::Null) => &Value::Array(Vec::new()),
            Some(results) => results,
            None => return Err(String::from("The Data API response has no `results`")),
        },
        (QueryType::Collection, OutputType::TotalCount) => node
            .get("totalCount")
            .ok_or("The Data API response has no `totalCount`")?,
    };

    serde_json::to_string(value).map_err(|error| error.to_string())
}

/// Builds the query, sends it with `send`, and returns the requested value as JSON text.
///
/// The Data API returns at most `PAGE_SIZE` records per request, so a collection of results with
/// a larger `take` is fetched page by page (`skip` moves on by one page each time) and returned
/// as one list. It stops early when a page comes back with fewer records than asked for.
pub fn run(
    params: &Params,
    mut send: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut send_query =
        |built: &BuiltQuery| send(&built.query).map_err(|error| host_error_message(&error));

    let total = requested_take(params);
    let paged = params.query_type == QueryType::Collection
        && params.output_type == OutputType::Results
        && total > PAGE_SIZE;

    // Pages are separate requests, so without a sort order a record can come back twice or not
    // at all. Sort by id when the developer gave no sort field.
    let has_sort = params
        .sort_field
        .is_some_and(|field| !field.trim().is_empty());
    let params = if paged && !has_sort {
        Params {
            sort_field: Some("id"),
            ..*params
        }
    } else {
        *params
    };
    let params = &params;

    let built = build_query(params)?;
    let response = send_query(&built)?;
    let first = extract_result(&response, &built, params)?;
    if !paged {
        return Ok(first);
    }

    let mut records = into_list(&first)?;
    let start = params.skip.unwrap_or(0);
    // The first request asked for a full page. Keep going while pages come back full.
    let mut last_page_was_full = records.len() as u32 == PAGE_SIZE;
    while last_page_was_full && (records.len() as u32) < total {
        let skip = start
            .checked_add(records.len() as u32)
            .ok_or("Skip is too large")?;
        let page_take = PAGE_SIZE.min(total - records.len() as u32);
        let built = build_page(params, skip, page_take)?;
        let response = send_query(&built)?;
        let page = into_list(&extract_result(&response, &built, params)?)?;
        last_page_was_full = page.len() as u32 == page_take;
        records.extend(page);
    }

    serde_json::to_string(&records).map_err(|error| error.to_string())
}

fn into_list(json: &str) -> Result<Vec<Value>, String> {
    match serde_json::from_str(json) {
        Ok(Value::Array(items)) => Ok(items),
        _ => Err(String::from("The Data API results are not a list")),
    }
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
    fn the_first_request_takes_at_most_200() {
        let vars = json!({});
        let mut p = params(&vars);
        for (take, expected) in [
            (None, 200),
            (Some(0), 200),
            (Some(50), 50),
            (Some(300), 200),
            (Some(9999), 200),
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

    type ReceivedQueries = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// A fake Data API with `count` records (ids 1..=count) that answers `skip`/`take` queries like
    /// the real one, including rejecting a `take` above 200. Returns the queries it received.
    fn fake_data_api(count: u32) -> (impl FnMut(&str) -> Result<String, String>, ReceivedQueries) {
        fake_data_api_at(0, count)
    }

    /// Like `fake_data_api`, but the `count` records start at id `first_id + 1`, and requests are
    /// answered as if they started at skip `first_id`.
    fn fake_data_api_at(
        first_id: u32,
        count: u32,
    ) -> (impl FnMut(&str) -> Result<String, String>, ReceivedQueries) {
        let received = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = received.clone();
        let send = move |query: &str| {
            log.borrow_mut().push(squash(query));
            let number_after = |label: &str| -> u32 {
                let rest = &query[query.find(label).unwrap() + label.len()..];
                rest.trim_start()
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap()
            };
            let (skip, take) = (number_after("skip:"), number_after("take:"));
            if take > 200 {
                return Err(String::from("unsupported request"));
            }
            let offset = skip.saturating_sub(first_id);
            let results: Vec<Value> = (offset + 1..=count.min(offset + take))
                .map(|id| json!({ "id": u64::from(first_id) + u64::from(id) }))
                .collect();
            Ok(json!({ "data": { "allWebuser": { "results": results } } }).to_string())
        };
        (send, received)
    }

    fn ids(json: &str) -> Vec<u64> {
        serde_json::from_str::<Vec<Value>>(json)
            .unwrap()
            .iter()
            .map(|record| record["id"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn a_take_above_200_is_fetched_in_pages_of_200() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(450);
        let (send, received) = fake_data_api(1000);
        let result = run(&p, send).unwrap();

        assert_eq!(ids(&result), (1..=450).collect::<Vec<_>>());
        let queries = received.borrow();
        assert_eq!(queries.len(), 3);
        assert!(queries[0].contains("skip: 0, take: 200"), "{}", queries[0]);
        assert!(
            queries[1].contains("skip: 200, take: 200"),
            "{}",
            queries[1]
        );
        assert!(queries[2].contains("skip: 400, take: 50"), "{}", queries[2]);
    }

    #[test]
    fn paging_starts_at_the_given_skip() {
        let vars = json!({});
        let mut p = params(&vars);
        p.skip = Some(30);
        p.take = Some(300);
        let (send, received) = fake_data_api(1000);
        let result = run(&p, send).unwrap();

        assert_eq!(ids(&result), (31..=330).collect::<Vec<_>>());
        let queries = received.borrow();
        assert!(queries[0].contains("skip: 30, take: 200"), "{}", queries[0]);
        assert!(
            queries[1].contains("skip: 230, take: 100"),
            "{}",
            queries[1]
        );
    }

    #[test]
    fn paging_stops_when_the_data_runs_out() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(1000);
        let (send, received) = fake_data_api(250);
        assert_eq!(ids(&run(&p, send).unwrap()).len(), 250);
        assert_eq!(received.borrow().len(), 2);

        // Exactly a multiple of the page size: one extra, empty request to find out.
        let (send, received) = fake_data_api(400);
        assert_eq!(ids(&run(&p, send).unwrap()).len(), 400);
        assert_eq!(received.borrow().len(), 3);
    }

    #[test]
    fn up_to_200_records_take_a_single_request() {
        let vars = json!({});
        let mut p = params(&vars);
        for take in [None, Some(0), Some(1), Some(200)] {
            p.take = take;
            let (send, received) = fake_data_api(1000);
            let result = run(&p, send).unwrap();
            assert_eq!(
                ids(&result).len(),
                take.filter(|t| *t > 0).unwrap_or(200) as usize
            );
            assert_eq!(received.borrow().len(), 1);
        }
    }

    #[test]
    fn take_is_capped_at_5000_records_in_25_requests() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(99_999);
        let (send, received) = fake_data_api(100_000);
        assert_eq!(ids(&run(&p, send).unwrap()).len(), 5000);
        assert_eq!(received.borrow().len(), 25);
    }

    #[test]
    fn total_count_and_records_never_ask_for_more_than_200() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(5000);
        p.output_type = OutputType::TotalCount;
        let built = build_query(&p).unwrap();
        assert!(built.query.contains("take: 200"), "{}", built.query);
        p.query_type = QueryType::Record;
        assert!(build_query(&p).unwrap().query.contains("take: 200"));
    }

    #[test]
    fn an_error_on_a_later_page_fails_the_step() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(500);
        let mut calls = 0;
        let error = run(&p, |_| {
            calls += 1;
            if calls == 1 {
                let results: Vec<Value> = (1..=200).map(|id| json!({ "id": id })).collect();
                Ok(json!({ "data": { "allWebuser": { "results": results } } }).to_string())
            } else {
                Err(String::from(r#"{"errors":[{"message":"timeout"}]}"#))
            }
        })
        .unwrap_err();
        assert_eq!(error, "timeout");
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

    fn selection_of(query_type: QueryType, output_type: OutputType, query_data: &str) -> String {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_type = query_type;
        p.output_type = output_type;
        p.query_data = Some(query_data);
        squash(&build_query(&p).unwrap().query)
    }

    #[test]
    fn id_is_added_to_the_selection_when_missing() {
        assert!(
            selection_of(QueryType::Collection, OutputType::Results, "email name")
                .ends_with("{ results { id email name } } }")
        );
        assert!(
            selection_of(QueryType::Record, OutputType::Results, "email")
                .ends_with("{ id email } }")
        );
    }

    #[test]
    fn id_is_not_added_twice_or_when_selected_anywhere_at_the_top_level() {
        for selection in [
            "id name",
            "name id",
            "name, id",
            "name\n id\n company { name }",
        ] {
            let query = selection_of(QueryType::Collection, OutputType::Results, selection);
            assert!(
                query.contains(&format!("results {{ {} }}", squash(selection))),
                "{query}"
            );
        }
    }

    #[test]
    fn an_id_inside_a_relation_does_not_count_as_the_top_level_id() {
        let query = selection_of(
            QueryType::Collection,
            OutputType::Results,
            "name company { id name }",
        );
        assert!(
            query.contains("results { id name company { id name } }"),
            "{query}"
        );
    }

    #[test]
    fn names_that_merely_contain_id_do_not_count() {
        let query = selection_of(
            QueryType::Collection,
            OutputType::Results,
            "identifier user_id",
        );
        assert!(
            query.contains("results { id identifier user_id }"),
            "{query}"
        );
    }

    #[test]
    fn total_count_does_not_get_an_id() {
        let query = selection_of(QueryType::Collection, OutputType::TotalCount, "name");
        assert!(query.ends_with("{ totalCount } }"), "{query}");
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
    fn a_closing_brace_in_query_data_is_reported_without_the_filter_hint() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("id name }");
        let error = build_query(&p).unwrap_err();
        assert!(error.starts_with("Query data has a closing `}`"), "{error}");
        assert!(!error.contains("filter"), "{error}");
        assert!(error.contains("on line 1 (`id name }`)"), "{error}");
    }

    #[test]
    fn a_stray_closing_brace_names_its_line() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("name\n  company { name }\n  email\n}");
        let error = build_query(&p).unwrap_err();
        assert!(error.contains("on line 4 (`}`)"), "{error}");
    }

    #[test]
    fn graphql_comments_are_understood() {
        let vars = json!({});
        let mut p = params(&vars);
        // Brackets and quotes inside a comment do not count, and an `id` in a comment is not the id.
        p.query_data =
            Some("# keep } and \" out of it\n  # the id of the person\n  name\n  company { name }");
        let query = build_query(&p).unwrap().query;
        assert!(query.contains("results { id # keep"), "{query}");
    }

    #[test]
    fn a_comment_on_the_last_line_does_not_swallow_the_closing_braces() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("id name # last property");
        p.filter = Some("id: { eq: 1 } # only one");
        let query = build_query(&p).unwrap().query;
        // Every comment is followed by a line break before the next closing brace.
        assert!(query.contains("# only one\n}"), "{query}");
        assert!(query.contains("# last property\n}"), "{query}");
    }

    #[test]
    fn a_mismatched_bracket_names_both_lines() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("company { name\n]");
        let error = build_query(&p).unwrap_err();
        assert!(error.contains("`]` on line 2"), "{error}");
        assert!(error.contains("`{` opened on line 1"), "{error}");
    }

    #[test]
    fn unbalanced_query_data_is_reported() {
        let vars = json!({});
        let mut p = params(&vars);
        p.query_data = Some("id company { id");
        assert!(build_query(&p).unwrap_err().contains("Query data"));
    }

    fn failing_build(query_data: &str, filter: Option<&str>, variables: &Value) -> String {
        let mut p = params(variables);
        p.query_data = Some(query_data);
        p.filter = filter;
        build_query(&p).unwrap_err()
    }

    #[test]
    fn a_value_ending_in_a_backslash_cannot_escape_its_string() {
        let vars = json!({ "a": "\\", "b": "} OR: [{ id: { gt: 0 } }] #" });
        let mut p = params(&vars);
        p.filter = Some("owner: { eq: \"{{a}}\" }, name: \"{{b}}\"");
        let query = build_query(&p).unwrap().query;

        // The backslash is doubled, so it is a backslash and the string still closes.
        assert!(
            query.contains("owner: { eq: \"\\\\\" }, name: \"} OR"),
            "{query}"
        );
        // Everything the second value contains stays inside its string: none of it is code.
        let code: String = crate::graphql::code_chars(&query)
            .unwrap()
            .into_iter()
            .map(|(_, character)| character)
            .collect();
        assert!(!code.contains("OR"), "{code}");
    }

    #[test]
    fn a_bare_carriage_return_cannot_end_a_comment_unnoticed() {
        let vars = json!({ "id": "1 #\r} }, skip: 0, take: 1) { results { id passwordHash } } other: allWebuser(where: { x: { eq: 1 \n" });
        let error = failing_build("id", Some("id: { eq: {{id}} }"), &vars);
        assert!(error.starts_with("Filter has a closing `}`"), "{error}");

        let error = failing_build("name # c\r}", None, &json!({}));
        assert!(error.starts_with("Query data has a closing `}`"), "{error}");
    }

    #[test]
    fn block_strings_cannot_hide_brackets() {
        let error = failing_build("name \"\"\"a(a\" \"\"\"{{}", None, &json!({}));
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn an_unterminated_string_is_reported() {
        let error = failing_build("id", Some("name: { eq: \"oops }"), &json!({}));
        assert!(error.contains("string that is never closed"), "{error}");
    }

    #[test]
    fn the_sort_field_and_the_model_name_are_validated() {
        let vars = json!({});
        let mut p = params(&vars);
        p.sort_field = Some(
            "id }) { results { id passwordHash } } other: allWebuser(where: {}, sort: { field: id",
        );
        let error = build_query(&p).unwrap_err();
        assert!(error.contains("sort field"), "{error}");

        p.sort_field = Some("company.name");
        assert!(
            build_query(&p)
                .unwrap()
                .query
                .contains("field: company.name")
        );

        p.sort_field = None;
        p.model_name = "Web user }";
        assert!(build_query(&p).unwrap_err().contains("model name"));
    }

    #[test]
    fn line_numbers_count_leading_blank_lines() {
        let error = failing_build("\n\n  name\n  email\n}", None, &json!({}));
        assert!(error.contains("on line 5 (`}`)"), "{error}");
    }

    #[test]
    fn filter_errors_do_not_echo_the_text_and_say_the_lines_are_rendered() {
        let vars = json!({ "secret": "abc123TOKEN" });
        let error = failing_build("id", Some("id: { eq: \"{{secret}}\" } }"), &vars);
        assert!(!error.contains("abc123TOKEN"), "{error}");
        assert!(!error.contains("`}`)"), "{error}");
        assert!(
            error.contains("counted after the variables were filled in"),
            "{error}"
        );

        // No tags in the filter: the line numbers are the typed ones, nothing to add.
        let error = failing_build("id", Some("id: { eq: 1 } }"), &vars);
        assert!(!error.contains("filled in"), "{error}");
    }

    #[test]
    fn query_data_is_not_checked_when_it_is_not_used() {
        let vars = json!({});
        let mut p = params(&vars);
        p.output_type = OutputType::TotalCount;
        p.query_data = Some("id name }");
        assert!(build_query(&p).is_ok());
    }

    #[test]
    fn null_results_are_an_empty_list_for_one_page_and_for_several() {
        let vars = json!({});
        let null_results =
            |_: &str| Ok(json!({ "data": { "allWebuser": { "results": null } } }).to_string());
        let mut p = params(&vars);
        p.take = Some(50);
        assert_eq!(run(&p, null_results).unwrap(), "[]");
        p.take = Some(500);
        assert_eq!(run(&p, null_results).unwrap(), "[]");
    }

    #[test]
    fn a_skip_that_overflows_is_an_error_not_a_panic() {
        let vars = json!({});
        let mut p = params(&vars);
        p.skip = Some(u32::MAX - 5);
        p.take = Some(500);
        let (send, _) = fake_data_api_at(u32::MAX - 5, 1000);
        assert_eq!(run(&p, send).unwrap_err(), "Skip is too large");
    }

    #[test]
    fn paging_without_a_sort_field_sorts_by_id_so_pages_do_not_overlap() {
        let vars = json!({});
        let mut p = params(&vars);
        p.take = Some(450);
        let (send, received) = fake_data_api(1000);
        run(&p, send).unwrap();
        assert!(
            received
                .borrow()
                .iter()
                .all(|query| query.contains("sort: { order: ASC, field: id }"))
        );

        // A sort field the developer chose is kept, and a single page gets no sort at all.
        p.sort_field = Some("name");
        let (send, received) = fake_data_api(1000);
        run(&p, send).unwrap();
        assert!(
            received
                .borrow()
                .iter()
                .all(|query| query.contains("field: name }"))
        );

        p.sort_field = None;
        p.take = Some(100);
        let (send, received) = fake_data_api(1000);
        run(&p, send).unwrap();
        assert!(!received.borrow()[0].contains("sort"));
    }

    /// The query the original ActionsJS step 1.7 builds (see tests/generate_query_cases.mjs).
    #[test]
    fn builds_the_same_queries_as_the_original_actionsjs_step() {
        // Update this when tests/generate_query_cases.mjs gets more cases, so a regenerated file
        // that quietly lost cases does not pass.
        const EXPECTED_CASES: usize = 14;

        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/data_api_request_cases.json")).unwrap();
        assert_eq!(cases.len(), EXPECTED_CASES);

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

            let built = build_query(&params).unwrap();
            assert_eq!(
                squash(&built.query),
                text("expectedQuery").unwrap(),
                "case {case}"
            );
            assert_eq!(built.field, text("expectedField").unwrap(), "case {case}");

            // What the step returns for the mocked Data API answer.
            let result = extract_result(text("response").unwrap(), &built, &params).unwrap();
            let result: Value = serde_json::from_str(&result).unwrap();
            assert_eq!(result, case["expectedResult"], "case {case}");
        }
    }
}
