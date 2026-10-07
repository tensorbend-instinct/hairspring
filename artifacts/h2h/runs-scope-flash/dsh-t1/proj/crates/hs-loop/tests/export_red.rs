use hs_loop::export::export_zip;

#[test]
fn x1_zip_contains_all_files_and_verifies() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("sess");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.log"), "alpha\n".repeat(100)).unwrap();
    std::fs::write(src.join("sub/b.json"), "{\"k\":1}").unwrap();
    std::fs::write(src.join("empty.txt"), "").unwrap();
    let out = d.path().join("out.zip");
    let n = export_zip(&src, &out).unwrap();
    assert_eq!(n, 3);
    // an independent reader (python zipfile) must accept it and read identical bytes
    let py = "import zipfile,sys;z=zipfile.ZipFile(sys.argv[1]);assert z.testzip() is None;print(sorted(z.namelist()));print(len(z.read('a.log')));print(z.read('sub/b.json').decode())";
    let o = std::process::Command::new("python3").args(["-c", py]).arg(&out).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(s.contains("['a.log', 'empty.txt', 'sub/b.json']"), "{s}");
    assert!(s.contains("600") && s.contains("{\"k\":1}"), "{s}");
}

#[test]
fn x2_missing_source_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(export_zip(&d.path().join("nope"), &d.path().join("o.zip")).is_err());
}
