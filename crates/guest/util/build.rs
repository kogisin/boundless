// Copyright (c) 2024 RISC Zero, Inc.
//
// All rights reserved.

use std::{collections::HashMap, env, path::PathBuf};

use risc0_build::{embed_methods_with_options, DockerOptions, GuestOptions};
use risc0_build_ethereum::generate_solidity_files;

// Paths where the generated Solidity files will be written.
const SOLIDITY_IMAGE_ID_PATH: &str = "../../../contracts/src/UtilImageID.sol";
const SOLIDITY_ELF_PATH: &str = "../../../contracts/test/UtilElf.sol";

fn main() {
    // Builds can be made deterministic, and thereby reproducible, by using Docker to build the
    // guest. Check the RISC0_USE_DOCKER variable and use Docker to build the guest if set.
    println!("cargo:rerun-if-env-changed=RISC0_USE_DOCKER");
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let use_docker = env::var("RISC0_USE_DOCKER")
        .ok()
        .map(|_| DockerOptions { root_dir: Some(manifest_dir.join("../../..")) });

    // Generate Rust source files for the methods crate.
    let guests = embed_methods_with_options(HashMap::from([
        ("echo", GuestOptions { features: Vec::new(), use_docker: use_docker.clone() }),
        ("identity", GuestOptions { features: Vec::new(), use_docker: use_docker.clone() }),
    ]));

    // Generate Solidity source files for use with Forge.
    let solidity_opts = risc0_build_ethereum::Options::default()
        .with_image_id_sol_path(SOLIDITY_IMAGE_ID_PATH)
        .with_elf_sol_path(SOLIDITY_ELF_PATH);

    if let Err(e) = generate_solidity_files(guests.as_slice(), &solidity_opts) {
        println!("cargo:warning=Failed to generate Solidity files: {}", e);
    };
}
