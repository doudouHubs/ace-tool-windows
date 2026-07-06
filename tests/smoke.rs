use rust_win32::enhancer::provider::EnhanceProviderKind;
use rust_win32::index::SearchProviderKind;

#[test]
fn public_provider_parsers_accept_plugin_cli_modes() {
    assert_eq!(SearchProviderKind::parse("remote"), Some(SearchProviderKind::Remote));
    assert_eq!(SearchProviderKind::parse("local"), Some(SearchProviderKind::Local));
    assert_eq!(
        EnhanceProviderKind::parse("codex"),
        Some(EnhanceProviderKind::Codex)
    );
}
