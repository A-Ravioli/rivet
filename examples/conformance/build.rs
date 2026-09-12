fn main() {
    if std::env::var_os("CARGO_FEATURE_VERILATOR").is_some() {
        rivet_verilator::Build::from_manifest().build();
    }
}
