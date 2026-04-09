use uuid::Uuid;

fn main() {
    let id = "";
    let safe_id = if id.trim().is_empty() {
        format!("call_{}", Uuid::new_v4().simple())
    } else {
        id.to_string()
    };
    println!("Safe ID: {}", safe_id);
}
