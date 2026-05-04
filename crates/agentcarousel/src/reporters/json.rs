use agentcarousel_core::Run;

pub fn print_json(run: &Run) {
    let payload = serde_json::to_string_pretty(run).unwrap_or_else(|_| "{}".to_string());
    println!("{payload}");
}
