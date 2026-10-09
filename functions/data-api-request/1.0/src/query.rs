//! Builds the Data API query from the step's options and picks the requested value out of the
//! response. Pure functions, so everything here is testable without a host.

use serde_json::Value;

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

    check_balanced("Query data", query_data)?;

    // Later steps (an update, a loop, ...) find a record by its id, so the id is always selected.
    let query_data = if needs_query_data && !selects_id(query_data) {
        format!("id {query_data}")
    } else {
        query_data.to_string()
    };

    let body = match (params.query_type, params.output_type) {
        (QueryType::Record, _) => query_data,
        (QueryType::Collection, OutputType::Results) => format!("results {{ {query_data}\n}}"),
        (QueryType::Collection, OutputType::TotalCount) => String::from("totalCount"),
    };

    let filter = template::render(params.filter.unwrap_or_default(), params.variables)?;
    let filter = normalize_filter(&filter)?;
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
        "query {{ {field}(where: {{ {filter}\n}}, skip: {skip}, take: {take}{sort}) {{ {body}\n}} }}"
    );

    Ok(BuiltQuery { query, field })
}

/// The characters of GraphQL text that are code: not inside a "string" and not inside a `#`
/// comment (which runs to the end of the line). A string is reported as one space.
fn code_chars(text: &str) -> Vec<(usize, char)> {
    let mut chars = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;

    for (index, character) in text.char_indices() {
        if in_comment {
            if character == '\n' {
                in_comment = false;
                chars.push((index, character));
            }
            continue;
        }
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
            '"' => {
                in_string = true;
                chars.push((index, ' '));
            }
            '#' => in_comment = true,
            _ => chars.push((index, character)),
        }
    }
    chars
}

/// Whether a selection like `id name company { id }` selects `id` at its top level (an `id`
/// inside a relation or an argument does not count).
fn selects_id(selection: &str) -> bool {
    let mut depth = 0i32;
    let mut word = String::new();
    let mut top_level_words = Vec::new();

    for (_, character) in code_chars(selection) {
        if depth == 0 && (character.is_alphanumeric() || character == '_') {
            word.push(character);
            continue;
        }
        if !word.is_empty() {
            top_level_words.push(std::mem::take(&mut word));
        }
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            _ => {}
        }
    }
    if !word.is_empty() {
        top_level_words.push(word);
    }

    top_level_words.iter().any(|word| word == "id")
}

enum BracketProblem {
    /// A closing bracket with nothing open.
    Unmatched { close: char, at: usize },
    /// A closing bracket of another kind than the one that is open.
    Mismatch {
        close: char,
        close_at: usize,
        open: char,
        open_at: usize,
    },
    /// An opening bracket that is never closed.
    Unclosed { open: char, at: usize },
}

fn find_bracket_problem(text: &str) -> Option<BracketProblem> {
    let mut open: Vec<(char, usize)> = Vec::new();

    for (index, character) in code_chars(text) {
        match character {
            '{' | '[' | '(' => open.push((character, index)),
            '}' | ']' | ')' => match open.pop() {
                None => {
                    return Some(BracketProblem::Unmatched {
                        close: character,
                        at: index,
                    });
                }
                Some((opener, open_at)) => {
                    let expected = match opener {
                        '{' => '}',
                        '[' => ']',
                        _ => ')',
                    };
                    if character != expected {
                        return Some(BracketProblem::Mismatch {
                            close: character,
                            close_at: index,
                            open: opener,
                            open_at,
                        });
                    }
                }
            },
            _ => {}
        }
    }
    open.pop()
        .map(|(opener, at)| BracketProblem::Unclosed { open: opener, at })
}

/// "line 12 (`  organization { name }`)": where a problem is, since the text can be long.
fn describe_line(text: &str, index: usize) -> String {
    let line_number = text[..index].matches('\n').count() + 1;
    let line = text.lines().nth(line_number - 1).unwrap_or_default().trim();
    let shown: String = line.chars().take(60).collect();
    let ellipsis = if line.chars().count() > 60 { "..." } else { "" };
    format!("line {line_number} (`{shown}{ellipsis}`)")
}

/// Fails with a readable message that points at the line when brackets do not pair up, instead
/// of sending the query and getting the Data API's "syntax error before: '}'".
fn check_balanced(label: &str, text: &str) -> Result<(), String> {
    let hint = if label == "Filter" {
        " The filter is wrapped in curly braces for you, so write it without outer braces, \
         for example: id: { eq: 1 }."
    } else {
        ""
    };

    match find_bracket_problem(text) {
        None => Ok(()),
        Some(BracketProblem::Unmatched { close, at }) => Err(format!(
            "{label} has a closing `{close}` without an opening one on {}.{hint}",
            describe_line(text, at)
        )),
        Some(BracketProblem::Mismatch {
            close,
            close_at,
            open,
            open_at,
        }) => Err(format!(
            "{label} has a closing `{close}` on {} that does not match the `{open}` opened on {}.{hint}",
            describe_line(text, close_at),
            describe_line(text, open_at)
        )),
        Some(BracketProblem::Unclosed { open, at }) => Err(format!(
            "{label} has an opening `{open}` on {} that is never closed.{hint}",
            describe_line(text, at)
        )),
    }
}

/// The text between the outer curly braces if the whole of `text` (ignoring comments and
/// whitespace) is one `{ ... }` pair, else `None`.
fn inside_outer_braces(text: &str) -> Option<&str> {
    let chars = code_chars(text);
    let mut significant = chars
        .iter()
        .filter(|(_, character)| !character.is_whitespace());
    let &(first_at, first) = significant.next()?;
    if first != '{' {
        return None;
    }

    let mut depth = 0i32;
    let mut close_at = None;
    for &(index, character) in chars.iter().filter(|(index, _)| *index >= first_at) {
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    close_at = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let close_at = close_at?;

    // Nothing but whitespace and comments may follow the closing brace.
    let nothing_after = chars
        .iter()
        .filter(|(index, _)| *index > close_at)
        .all(|(_, character)| character.is_whitespace());
    nothing_after.then(|| text[first_at + 1..close_at].trim())
}

/// Checks the rendered filter and removes one pair of outer curly braces if the whole filter is
/// wrapped in them (`{ id: { eq: 1 } }`), because the step adds those itself. A filter can never
/// validly start with `{`, so this cannot change the meaning of a working filter.
fn normalize_filter(filter: &str) -> Result<String, String> {
    check_balanced("Filter", filter)?;
    let trimmed = filter.trim();
    Ok(inside_outer_braces(trimmed).unwrap_or(trimmed).to_string())
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

    let built = build_query(params)?;
    let response = send_query(&built)?;
    let first = extract_result(&response, &built, params)?;

    let paged =
        params.query_type == QueryType::Collection && params.output_type == OutputType::Results;
    let total = requested_take(params);
    if !paged || total <= PAGE_SIZE {
        return Ok(first);
    }

    let mut records = into_list(&first)?;
    let start = params.skip.unwrap_or(0);
    // The first request asked for a full page. Keep going while pages come back full.
    let mut last_page_was_full = records.len() as u32 == PAGE_SIZE;
    while last_page_was_full && (records.len() as u32) < total {
        let skip = start + records.len() as u32;
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
            let results: Vec<Value> = (skip + 1..=count.min(skip + take))
                .map(|id| json!({ "id": id }))
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
