use serde_json::Value;

pub fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(std::string::ToString::to_string)
}

pub fn u64_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key)?.as_u64()
}

pub fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key)?.as_u64().map(|v| usize::try_from(v).unwrap_or(usize::MAX))
}
