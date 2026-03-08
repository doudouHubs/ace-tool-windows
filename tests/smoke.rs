use rust_win32::mcp::schemas;

#[test]
fn tools_list_contains_expected_tools() {
  let tools = schemas::tool_list();
  let names: Vec<String> = tools
    .iter()
    .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
    .collect();

  assert!(names.contains(&"search_context".to_string()));
  assert!(names.contains(&"enhance_prompt".to_string()));
  assert!(names.contains(&"enhancer".to_string()));
}
