//! Repository index signer for LobsterOS package manager
//!
//! This tool signs a repository index.json file with an Ed25519 private key.
//! Run with: cargo run --bin sign-index -- <index.json> <private_key_hex> <output.json>
//!
//! The private key should be 64 hex characters (32 bytes).
//!
//! The signature is added as a top-level "signature" field in the JSON.

use std::fs;
use std::io::Write;
use ed25519_dalek::{Signer, SigningKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: sign-index <index.json> <private_key_hex> <output.json>");
        eprintln!("  private_key_hex: 64 hex characters (32 bytes) Ed25519 private key");
        std::process::exit(1);
    }

    let index_path = &args[1];
    let private_key_hex = &args[2];
    let output_path = &args[3];

    // Parse private key
    if private_key_hex.len() != 64 {
        eprintln!("Error: Private key must be 64 hex characters (32 bytes)");
        std::process::exit(1);
    }

    let mut key_bytes = [0u8; 32];
    for i in 0..32 {
        let byte_str = &private_key_hex[i * 2..i * 2 + 2];
        key_bytes[i] = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("Invalid hex at position {}", i * 2))?;
    }

    let signing_key = SigningKey::from_bytes(&key_bytes);

    // Read index JSON
    let index_content = fs::read_to_string(index_path)?;
    let json_str = index_content.trim();

    // Remove any existing signature field
    let json_without_sig = remove_signature_field(json_str).ok_or("Failed to remove existing signature")?;

    // Sign the canonical JSON (without signature field)
    let signature = signing_key.sign(json_without_sig.as_bytes());
    let sig_hex = signature.to_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    // Add signature field to JSON
    let signed_json = add_signature_field(&json_without_sig, &sig_hex)?;

    // Write output
    let mut out = fs::File::create(output_path)?;
    out.write_all(signed_json.as_bytes())?;
    out.flush()?;

    println!("Index signed successfully!");
    println!("  Input:  {}", index_path);
    println!("  Output: {}", output_path);
    println!("  Signature: {}", sig_hex);

    Ok(())
}

/// Remove the "signature" field from JSON for signing
fn remove_signature_field(json: &str) -> Option<String> {
    let sig_field = "\"signature\"";
    let sig_start = json.find(sig_field)?;
    let field_start = sig_start;

    // Find the colon after the field name
    let colon_pos = json[field_start..].find(':')? + field_start;
    let value_start = json[colon_pos + 1..].trim_start();

    if !value_start.starts_with('"') {
        return None;
    }

    let quote_end = value_start[1..].find('"')? + 1;
    let value_part = &json[colon_pos + 1..];
    let value_trimmed = value_part.trim_start();
    let field_end = colon_pos + 1 + (value_trimmed.as_ptr() as usize - value_part.as_ptr() as usize) + quote_end + 1;

    // Build result without the signature field
    let mut result = String::with_capacity(json.len());
    result.push_str(&json[..field_start]);

    // Check for trailing comma
    let after_field = &json[field_end..].trim_start();
    let after_field = if after_field.starts_with(',') {
        &after_field[1..]
    } else {
        after_field
    };
    result.push_str(after_field);

    Some(result.trim().to_string())
}

/// Add signature field to JSON
fn add_signature_field(json: &str, signature: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Find the last '}' to insert before it
    let last_brace = json.rfind('}').ok_or("Invalid JSON: no closing brace")?;

    let mut result = String::with_capacity(json.len() + 100);
    result.push_str(&json[..last_brace]);
    result.push_str(",\n  \"signature\": \"");
    result.push_str(signature);
    result.push_str("\"\n}");

    Ok(result)
}
