use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct ModNode {
    file: Option<String>,
    children: BTreeMap<String, ModNode>,
}

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_protos(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "proto") {
            out.push(path);
        }
    }
}

fn insert(root: &mut ModNode, package: &str) {
    let mut node = root;
    for segment in package.split('.') {
        node = node.children.entry(segment.to_string()).or_default();
    }
    node.file = Some(package.to_string());
}

fn render(node: &ModNode, buf: &mut String) {
    for (name, child) in &node.children {
        buf.push_str(&format!("pub mod {name} {{\n"));
        if let Some(file) = &child.file {
            buf.push_str(&format!(
                "include!(concat!(env!(\"OUT_DIR\"), \"/{file}.rs\"));\n"
            ));
        }
        render(child, buf);
        buf.push_str("}\n");
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let protos_root = manifest_dir.join("protos");

    let mut proto_files = Vec::new();
    collect_protos(&protos_root, &mut proto_files);
    proto_files.sort();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // prost never removes files from a previous run, so a package that gets renamed
    // or fixed (as opposed to added) would otherwise leave its old generated file
    // behind and silently pull it into the module tree below.
    for entry in fs::read_dir(&out_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            fs::remove_file(path).unwrap();
        }
    }

    // protoc gets invoked with every proto file as its own argument. On Windows that
    // command line (300+ absolute paths) exceeds CreateProcess's length limit and
    // fails with "os error 206" (filename or extension too long) - even with a
    // directly-invoked protoc.exe, not just a shim. protoc's own `@<file>` response-file
    // syntax sidesteps this: the whole list goes into one file, and protoc alone (not
    // the OS) reads it, so the actual process is started with a single short argument.
    let argfile_path = out_dir.join("protos.list");
    let argfile_contents = proto_files
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&argfile_path, argfile_contents).unwrap();

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&[format!("@{}", argfile_path.display())], &[protos_root])
        .expect("failed to compile Android Auto / OpenAuto protobufs");

    // prost writes one `<package>.rs` file per unique `package` declaration, and
    // packages nest arbitrarily (e.g. `aap_protobuf.service.mediabrowser` is both a
    // file on its own and the parent of `aap_protobuf.service.mediabrowser.message`),
    // so the module tree is derived from the generated filenames rather than
    // hand-written to match the .proto package hierarchy.
    let mut packages = Vec::new();
    for entry in fs::read_dir(&out_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                packages.push(stem.to_string());
            }
        }
    }

    let mut root = ModNode::default();
    for package in &packages {
        insert(&mut root, package);
    }

    let mut buf = String::new();
    render(&root, &mut buf);
    fs::write(out_dir.join("mod_tree.rs"), buf).unwrap();

    println!("cargo:rerun-if-changed=protos");
}
