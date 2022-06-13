fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["proto/chronik.proto"], &["proto"])?;
    Ok(())
}
