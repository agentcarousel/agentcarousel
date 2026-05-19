pub const INDEX_HTML: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/dashboard/index.html"));
pub const RUN_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/dashboard/run.html"));
pub const COMPARE_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/dashboard/compare.html"
));
pub const REVIEW_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/dashboard/review.html"
));
