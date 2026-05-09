/// Extracts the enclosing function name from the type name of a macro-local marker item.
pub fn method_name_from_marker_type_name(
    marker_type_name: &'static str,
    marker_name: &str,
) -> &'static str {
    let Some(mut enclosing_path) = marker_type_name
        .strip_suffix(marker_name)
        .and_then(|prefix| prefix.strip_suffix("::"))
    else {
        return marker_type_name;
    };

    // Logs are often emitted inside iterator or error-handling closures, but callers
    // want the owning function name rather than the synthetic closure segment.
    while let Some(parent_path) = enclosing_path.strip_suffix("::{{closure}}") {
        enclosing_path = parent_path;
    }

    enclosing_path.rsplit("::").next().unwrap_or(enclosing_path)
}
