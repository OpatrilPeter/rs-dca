
use assert_cmd::prelude::*;
use assert_fs::{prelude::*, TempDir};
use std::process::Command;

/// Basic test for the command line interface
#[test]
fn cli() {
    {
        Command::cargo_bin("dca").unwrap()
            .arg("--help")
            .assert()
            .success();
    }
    let contents: &[u8] = b"DCA\nnotes.txt\n8\nmy\nnotes\ndump.bin\n5\n\x12\x34\x56\0\0\nempty\n0\n\n";
    {
        let dir = TempDir::new().unwrap();

        dir.child("notes.txt").write_str("my\nnotes").unwrap();
        dir.child("dump.bin").write_binary(b"\x12\x34\x56\0\0").unwrap();
        dir.child("empty").touch().unwrap();

        Command::cargo_bin("dca").unwrap()
            .args(&[
                "--compress",
                "notes.txt",
                "dump.bin",
                "empty",
                "--output",
                "archive.dca"
            ])
            .current_dir(dir.path())
            .assert()
            .success();

        dir.child("archive.dca").assert(contents);
    }
    {
        let dir = TempDir::new().unwrap();

        dir.child("archive.dca").write_binary(contents).unwrap();

        Command::cargo_bin("dca").unwrap()
            .args(&[
                "--decompress",
                "archive.dca",
                "--output",
                "."
            ])
            .current_dir(dir.path())
            .assert()
            .success();

        dir.child("notes.txt").assert("my\nnotes");
        dir.child("dump.bin").assert(b"\x12\x34\x56\0\0" as &[u8]);
        dir.child("empty").assert("");
    }
}
