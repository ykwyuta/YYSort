use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output},
};
use yy_encoding::{Encoding, EscapeMode, encode_all};
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yysort"))
        .current_dir(dir)
        .args(args)
        .env_remove("YYSORT_WORKER_DIR")
        .env_remove("YYSORT_STAGED_OUTPUT")
        .output()
        .unwrap()
}
fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn overlay_reads_prior_items_and_overlapping_moves_preserve_other_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"abcdef").unwrap();
    for (items, expected) in [
        ("3:1,2,1:3,2", b"ababef".as_slice()),
        ("2:1,4", b"aabcdf"),
        ("3,2,C'!'", b"cd!def"),
        ("9:C'Z',7:9,1", b"abcdefZ Z"),
    ] {
        fs::write(
            p.join("cards"),
            format!("OPTION COPY\n INREC OVERLAY=({items})\n"),
        )
        .unwrap();
        ok(&run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--record-length",
                "6",
                "--control",
                "cards",
            ],
        ));
        assert_eq!(fs::read(p.join("out")).unwrap(), expected, "{items}");
    }
}

#[test]
fn build_output_columns_blanks_and_ebcdic_padding() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let enc = Encoding::from_name("IBM-037").unwrap();
    fs::write(
        p.join("in"),
        encode_all(enc, b"AB", EscapeMode::Reject).unwrap(),
    )
    .unwrap();
    fs::write(
        p.join("cards"),
        "OPTION COPY\n OUTREC FIELDS=(1,1,4:C'Z',2X,2,1)\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "2",
            "--encoding",
            "IBM-037",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(
        fs::read(p.join("out")).unwrap(),
        encode_all(enc, b"A  Z  B", EscapeMode::Reject).unwrap()
    );
}

#[test]
fn overlay_pipeline_outfil_and_field_conditions_see_current_layout() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"b12a23c34").unwrap();
    fs::write(p.join("cards"), "INREC OVERLAY=(4:1,1)\n SORT FIELDS=(4,1,CH,A)\n OUTREC OVERLAY=(2:C'!')\n OUTFIL FNAMES=EQ,INCLUDE=(1,1,CH,EQ,4,1,CH),OVERLAY=(3:2,1)\n OUTFIL FNAMES=NO,SAVE\n").unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "3",
            "--control",
            "cards",
            "--out",
            "EQ=equal",
            "--out",
            "NO=other",
        ],
    ));
    assert_eq!(fs::read(p.join("out")).unwrap(), b"a!3ab!2bc!4c");
    assert_eq!(fs::read(p.join("equal")).unwrap(), b"a!!ab!!bc!!c");
    assert!(fs::read(p.join("other")).unwrap().is_empty());
}

#[test]
fn field_comparisons_mixed_numeric_formats_and_format_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let mut input = vec![];
    for (a, b, name) in [(500u16, -2i16, b'A'), (1, 2, b'B'), (2, 2, b'C')] {
        input.extend(a.to_be_bytes());
        input.extend(b.to_be_bytes());
        input.push(name);
    }
    fs::write(p.join("in"), &input).unwrap();
    for expr in [
        "FORMAT=FI,COND=(1,2,BI,GT,3,2)",
        "COND=(1,2,BI,GT,3,2),FORMAT=FI",
        "COND=(1,2,BI,GT,3,2,FI)",
    ] {
        fs::write(p.join("cards"), format!("OPTION COPY\n INCLUDE {expr}\n")).unwrap();
        ok(&run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--record-length",
                "5",
                "--control",
                "cards",
            ],
        ));
        assert_eq!(fs::read(p.join("out")).unwrap(), &input[..5]);
    }
    fs::write(
        p.join("in"),
        [
            0x12, 0x3c, 0xf1, 0xf2, 0xc3, b'E', 0x12, 0x3d, 0xf1, 0xf2, 0xc3, b'N',
        ],
    )
    .unwrap();
    fs::write(
        p.join("cards"),
        "OPTION COPY\n INCLUDE COND=(1,2,PD,EQ,3,3,ZD)\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--overwrite",
            "--record-length",
            "6",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(
        fs::read(p.join("out")).unwrap(),
        [0x12, 0x3c, 0xf1, 0xf2, 0xc3, b'E']
    );
}

#[test]
fn character_field_comparison_operators_and_omit() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"aaaaaabbbbaa").unwrap();
    for (op, expected) in [
        ("EQ", b"aaaa".as_slice()),
        ("NE", b"aabbbbaa"),
        ("LT", b"aabb"),
        ("LE", b"aaaaaabb"),
        ("GT", b"bbaa"),
        ("GE", b"aaaabbaa"),
    ] {
        fs::write(
            p.join("cards"),
            format!("OPTION COPY\n INCLUDE COND=(1,2,CH,{op},3,2,CH)\n"),
        )
        .unwrap();
        ok(&run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--record-length",
                "4",
                "--control",
                "cards",
            ],
        ));
        assert_eq!(fs::read(p.join("out")).unwrap(), expected, "{op}");
    }
    fs::write(
        p.join("cards"),
        "OPTION COPY\n OMIT FORMAT=CH,COND=(1,2,EQ,3,2)\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--overwrite",
            "--record-length",
            "4",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(fs::read(p.join("out")).unwrap(), b"aabbbbaa");
}

#[test]
fn csv_field_comparisons_and_nested_default_format_conditions() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), "id,a,b\r\nx,1,2\r\ny,10,2\r\nz,-1,-1\r\n").unwrap();
    fs::write(
        p.join("cards"),
        "OPTION COPY\n INCLUDE FORMAT=NUM,COND=((2,1,GT,3,1),OR,(2,1,EQ,-1,AND,3,1,EQ,2,1))\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--format",
            "csv",
            "--header",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(
        fs::read_to_string(p.join("out")).unwrap(),
        "id,a,b\r\ny,10,2\r\nz,-1,-1\r\n"
    );
}

#[test]
fn invalid_reformat_and_comparisons_preserve_output_even_for_empty_input() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), []).unwrap();
    fs::write(p.join("out"), b"keep").unwrap();
    for card in [
        "INREC OVERLAY=(0:C'A')",
        "INREC BUILD=(4:C'X',1:C'Y')",
        "OUTREC OVERLAY=(999999999999999999:C'A')",
        "INREC OVERLAY=(2:)",
        "INREC OVERLAY=(1:C'X'),BUILD=(1,1)",
        "OUTFIL OVERLAY=(999:C'A')",
        "OUTFIL BUILD=(1,1),OVERLAY=(1:C'X')",
        "INCLUDE COND=(1,1,CH,EQ,2,1,BI)",
        "INCLUDE COND=(1,1,CH,EQ,2,2,CH)",
        "INCLUDE FORMAT=BI,FORMAT=FI,COND=(1,1,EQ,0)",
        "INCLUDE COND=(1,1,EQ,2,1)",
    ] {
        fs::write(p.join("cards"), format!("OPTION COPY\n {card}\n")).unwrap();
        let out = run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--record-length",
                "2",
                "--max-record",
                "32",
                "--control",
                "cards",
            ],
        );
        assert!(!out.status.success(), "{card}");
        assert_eq!(fs::read(p.join("out")).unwrap(), b"keep", "{card}");
    }
}
#[test]
fn fixed_stable_multi_key_sort_and_opaque_packed_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let input = [
        b'b', 2, 0xff, 0x12, 0x3c, b'a', 1, 0, 0x12, 0x3d, b'b', 1, 0x80, 0x45, 0x6c, b'b', 1,
        0x90, 0x12, 0x3c,
    ];
    fs::write(p.join("in"), input).unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "5",
            "--key",
            "1:1:CH:A",
            "--key",
            "2:1:BI:A",
        ],
    ));
    let out = fs::read(p.join("out")).unwrap();
    assert_eq!(
        out,
        [&input[5..10], &input[10..15], &input[15..20], &input[..5]].concat()
    );
    assert_eq!(fs::read(p.join("in")).unwrap(), input);
    assert_eq!(fs::read_dir(p).unwrap().count(), 2);
}
#[test]
fn csv_rfc_quotes_multiline_header_unicode_and_exact_decimal_sum() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"),"name,value,note\r\n乙,999999999999999999999999.25,\"x,y\r\nz\"\r\n甲,2,\"a\"\"b\"\r\n乙,0.75,last\r\n").unwrap();
    ok(&run(
        p,
        &[
            "in", "-o", "out", "--format", "csv", "--header", "--key", "1:TEXT:A", "--sum", "2:NUM",
        ],
    ));
    let out = fs::read_to_string(p.join("out")).unwrap();
    assert_eq!(
        out,
        "name,value,note\r\n乙,1000000000000000000000000,\"x,y\r\nz\"\r\n甲,2,\"a\"\"b\"\r\n"
    );
}
#[test]
fn csv_transcoding_reuses_editor_japanese_codecs() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    for (name, encoding) in [
        ("CP932", Encoding::Cp932),
        ("Shift_JIS-2004", Encoding::ShiftJis2004),
        ("EUC-JIS-2004", Encoding::EucJis2004),
        ("UTF-16LE", Encoding::Utf16Le),
        ("ISO-2022-JP", Encoding::Iso2022Jp),
    ] {
        let source =
            encode_all(encoding, "乙,2\r\n甲,1\r\n".as_bytes(), EscapeMode::Reject).unwrap();
        fs::write(p.join("in"), source).unwrap();
        ok(&run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--format",
                "csv",
                "--encoding",
                name,
                "--output-encoding",
                "UTF-8",
                "--key",
                "2:NUM:A",
            ],
        ));
        assert_eq!(
            fs::read_to_string(p.join("out")).unwrap(),
            "甲,1\r\n乙,2\r\n"
        );
    }
}
#[test]
fn ebcdic_raw_character_collation_and_pd_zd_fi_sum() {
    use yysort::keys::{Format, Number, encode_number};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    for (format, width, name) in [
        (Format::PD, 3, "PD"),
        (Format::ZD, 5, "ZD"),
        (Format::FI, 4, "FI"),
        (Format::BI, 4, "BI"),
    ] {
        let mut data = Vec::new();
        for (key, value) in [(0xc2, "100"), (0xc1, "200"), (0xc2, "50")] {
            data.push(key);
            data.extend(
                encode_number(
                    &Number::parse(value).unwrap(),
                    format,
                    width,
                    Encoding::Utf8,
                )
                .unwrap(),
            );
        }
        fs::write(p.join("in"), data).unwrap();
        ok(&run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--overwrite",
                "--encoding",
                "IBM-939",
                "--record-length",
                &(width + 1).to_string(),
                "--key",
                "1:1:CH:A",
                "--sum",
                &format!("2:{width}:{name}"),
            ],
        ));
        let mut expected = vec![0xc1];
        expected.extend(
            encode_number(
                &Number::parse("200").unwrap(),
                format,
                width,
                Encoding::Utf8,
            )
            .unwrap(),
        );
        expected.push(0xc2);
        expected.extend(
            encode_number(
                &Number::parse("150").unwrap(),
                format,
                width,
                Encoding::Utf8,
            )
            .unwrap(),
        );
        assert_eq!(fs::read(p.join("out")).unwrap(), expected);
    }
}
#[test]
fn dfsort_cards_filters_before_inrec_keys_after_inrec_and_outrec_after_sum() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"b2Yb1Na3Yb4Y").unwrap();
    fs::write(p.join("cards"),"* example\n INCLUDE COND=(3,1,CH,EQ,C'Y')\n INREC BUILD=(2,1,1,1)\n SORT FIELDS=(2,1,CH,A,\n 1,1,CH,D)\n OUTREC BUILD=(C'[',2,1,C':',1,1,C']')\n END\n").unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "3",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(fs::read(p.join("out")).unwrap(), b"[a:3][b:4][b:2]");
}
#[test]
fn merge_checks_input_order_and_deduplicates_stably() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("a"), b"a1c1").unwrap();
    fs::write(p.join("b"), b"a2b2").unwrap();
    ok(&run(
        p,
        &[
            "a",
            "b",
            "-o",
            "out",
            "--record-length",
            "2",
            "--key",
            "1:1:CH:A",
            "--merge",
            "--unique",
        ],
    ));
    assert_eq!(fs::read(p.join("out")).unwrap(), b"a1b2c1");
    fs::write(p.join("b"), b"b2a2").unwrap();
    assert!(
        !run(
            p,
            &[
                "a",
                "b",
                "-o",
                "out",
                "--overwrite",
                "--record-length",
                "2",
                "--key",
                "1:1:CH:A",
                "--merge"
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(p.join("out")).unwrap(), b"a1b2c1");
}
#[test]
fn malformed_input_unsupported_cards_limits_and_outputs_fail_without_partial_publication() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"abcd").unwrap();
    fs::write(p.join("out"), b"keep").unwrap();
    for extra in [
        vec!["--memory", "1MiB"],
        vec!["--temp-limit", "1B"],
        vec!["--max-record", "1B"],
        vec!["--key", "0:1:CH:A"],
        vec!["--key", "5:1:CH:A"],
    ] {
        let mut args = vec![
            "in",
            "-o",
            "out",
            "--overwrite",
            "--record-length",
            "2",
            "--key",
            "1:1:CH:A",
        ];
        args.extend(extra);
        assert!(!run(p, &args).status.success());
        assert_eq!(fs::read(p.join("out")).unwrap(), b"keep");
    }
    for card in [
        "OUTFIL FNAMES=X,UNKNOWN=1",
        "SORT FIELDS=(1,1,CH,A)\n OPTION NOEQUALS",
        "SORT FIELDS=(1,1,CH,A)\n INREC OVERLAY=(1,C'X')",
        "SORT FIELDS=(1,1,CH,A)\n JOINKEYS FILE=F1,FIELDS=(1,1,A)",
    ] {
        fs::write(p.join("cards"), card).unwrap();
        assert!(
            !run(
                p,
                &[
                    "in",
                    "-o",
                    "out",
                    "--overwrite",
                    "--record-length",
                    "2",
                    "--control",
                    "cards"
                ]
            )
            .status
            .success()
        );
        assert_eq!(fs::read(p.join("out")).unwrap(), b"keep");
    }
    assert!(
        !run(
            p,
            &[
                "in",
                "-o",
                "in",
                "--overwrite",
                "--record-length",
                "2",
                "--key",
                "1:1:CH:A"
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(p.join("in")).unwrap(), b"abcd");
    assert_eq!(fs::read_dir(p).unwrap().count(), 3);
}
#[test]
fn csv_invalid_quoting_bad_encodings_truncation_and_overflow_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    for bytes in [
        b"\"unterminated".as_slice(),
        b"a\"b,1\r\n",
        b"\"a\"x,1\r\n",
        b"a,1\nb,2\n",
        b"a,b\rc,d",
        b"a,\xff\r\n",
        b"a,b\r\nc\r\n",
    ] {
        fs::write(p.join("in"), bytes).unwrap();
        assert!(
            !run(
                p,
                &["in", "-o", "out", "--format", "csv", "--key", "1:TEXT:A"]
            )
            .status
            .success()
        );
        assert!(!p.join("out").exists());
    }
    fs::write(p.join("in"), b"abc").unwrap();
    assert!(
        !run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--record-length",
                "2",
                "--key",
                "1:1:CH:A"
            ]
        )
        .status
        .success()
    );
    fs::write(p.join("in"), [b'a', 255, b'a', 1]).unwrap();
    assert!(
        !run(
            p,
            &[
                "in",
                "-o",
                "out",
                "--record-length",
                "2",
                "--key",
                "1:1:CH:A",
                "--sum",
                "2:1:BI"
            ]
        )
        .status
        .success()
    );
    assert!(!p.join("out").exists());
}
#[test]
fn external_multi_pass_sort_is_stable_and_os_limits_are_installed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let count = 180_000u64;
    let mut input = fs::File::create(p.join("in")).unwrap();
    for i in 0..count {
        write!(input, "{:08}{:016}........", (count - i) % 701, i).unwrap();
    }
    drop(input);
    fs::write(
        p.join("control"),
        "INREC OVERLAY=(25:C'.......!')\n SORT FIELDS=(1,8,CH,A)\n OUTREC OVERLAY=(32:C'?')\n OUTFIL FNAMES=TOP,ENDREC=5\n",
    )
    .unwrap();
    let out = run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "32",
            "--control",
            "control",
            "--out",
            "TOP=top",
            "--memory",
            "128MiB",
            "--threads",
            "1",
            "--merge-fan-in",
            "2",
            "--stats",
        ],
    );
    ok(&out);
    let stats: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert!(stats["initial_runs"].as_u64().unwrap() >= 2);
    assert!(stats["merge_operations"].as_u64().unwrap() >= 1);
    assert_eq!(stats["resources"]["affinity_cpus"], 1);
    assert_eq!(
        stats["resources"]["memory_ceiling_bytes"],
        128 * 1024 * 1024
    );
    assert!(stats["peak_accounted_chunk_bytes"].as_u64().unwrap() <= 32 * 1024 * 1024);
    let mut file = fs::File::open(p.join("out")).unwrap();
    let mut previous = None;
    let mut seen = vec![false; count as usize];
    let mut record = [0; 32];
    for _ in 0..count {
        file.read_exact(&mut record).unwrap();
        let s = std::str::from_utf8(&record).unwrap();
        let key = s[..8].parse::<u64>().unwrap();
        let seq = s[8..24].parse::<u64>().unwrap();
        assert_eq!(&record[24..], b".......?");
        assert_eq!(key, (count - seq) % 701);
        assert!(!seen[seq as usize]);
        seen[seq as usize] = true;
        if let Some(old) = previous {
            assert!(old < (key, seq));
        }
        previous = Some((key, seq));
    }
    assert_eq!(file.read(&mut record).unwrap(), 0);
    assert!(seen.into_iter().all(|v| v));
    let mut first = [0u8; 160];
    fs::File::open(p.join("out"))
        .unwrap()
        .read_exact(&mut first)
        .unwrap();
    assert_eq!(fs::read(p.join("top")).unwrap(), first);
    assert_eq!(stats["output_files"]["TOP"], 5);
    assert_eq!(fs::read_dir(p).unwrap().count(), 4);
}

#[test]
fn custom_editor_mapping_is_loaded_and_csv_copy_can_transcode() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(
        p.join("map"),
        "@name test-mainframe\n@base IBM-037\nC1 U+2603\n",
    )
    .unwrap();
    fs::write(
        p.join("in"),
        [0xc2, 0x6b, 0xf2, 0x15, 0xc1, 0x6b, 0xf1, 0x15],
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--format",
            "csv",
            "--encoding",
            "test-mainframe",
            "--mapping",
            "map",
            "--output-encoding",
            "UTF-8",
            "--allow-lf",
            "--key",
            "2:NUM:A",
        ],
    ));
    assert_eq!(fs::read_to_string(p.join("out")).unwrap(), "☃,1\r\nB,2\r\n");
}
#[test]
fn copy_skip_stop_filters_and_empty_inputs_do_not_read_past_stop() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"a1b2c3X").unwrap();
    fs::write(
        p.join("cards"),
        "OPTION COPY,SKIPREC=1,STOPAFT=1\n INCLUDE COND=(2,1,CH,GT,C'2')\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "out",
            "--record-length",
            "2",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(fs::read(p.join("out")).unwrap(), b"c3");
    fs::write(p.join("in"), b"").unwrap();
    ok(&run(p, &["in", "-o", "empty", "--format", "csv", "--copy"]));
    assert!(fs::read(p.join("empty")).unwrap().is_empty());
}

#[test]
fn outfil_multiple_outputs_see_global_outrec_and_save_is_global() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"b1Ya2Yb3Na4Yc5Y").unwrap();
    fs::write(p.join("cards"),"SORT FIELDS=(1,1,CH,A)\n OUTREC BUILD=(3,1,1,2)\n OUTFIL FNAMES=REST,SAVE\n OUTFIL FNAMES=A,INCLUDE=(2,1,CH,EQ,C'a'),BUILD=(3,1,C':',2,1)\n OUTFIL FNAMES=B,INCLUDE=(1,1,CH,EQ,C'N')\n").unwrap();
    let out = run(
        p,
        &[
            "in",
            "-o",
            "main",
            "--record-length",
            "3",
            "--control",
            "cards",
            "--out",
            "rest=rest",
            "--out",
            "A=a",
            "--out",
            "B=b",
            "--stats",
        ],
    );
    ok(&out);
    assert_eq!(fs::read(p.join("main")).unwrap(), b"Ya2Ya4Yb1Nb3Yc5");
    assert_eq!(fs::read(p.join("a")).unwrap(), b"2:a4:a");
    assert_eq!(fs::read(p.join("b")).unwrap(), b"Nb3");
    assert_eq!(fs::read(p.join("rest")).unwrap(), b"Yb1Yc5");
    let stats: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(
        stats["output_files"],
        serde_json::json!({"A":2,"B":1,"REST":2,"SORTOUT":5})
    );
}
#[test]
fn outfil_ranges_sampling_accept_split_and_save_consider_all_groups() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"0123456789").unwrap();
    fs::write(p.join("cards"),"OPTION COPY\n OUTFIL FNAMES=REST,SAVE\n OUTFIL FNAMES=(A,B),STARTREC=2,ENDREC=9,SAMPLE=(3,2),ACCEPT=4,SPLITBY=2\n OUTFIL FNAMES=(C,D),INCLUDE=(1,1,CH,EQ,C'9')\n").unwrap();
    ok(&run(
        p,
        &[
            "in",
            "--record-length",
            "1",
            "--control",
            "cards",
            "--out",
            "A=a",
            "--out",
            "B=b",
            "--out",
            "C=c",
            "--out",
            "D=d",
            "--out",
            "REST=rest",
        ],
    ));
    for (name, expected) in [
        ("a", "12"),
        ("b", "45"),
        ("c", "9"),
        ("d", "9"),
        ("rest", "03678"),
    ] {
        assert_eq!(fs::read(p.join(name)).unwrap(), expected.as_bytes());
    }
    fs::write(
        p.join("cards"),
        "OPTION COPY\n OUTFIL FNAMES=(A,B,C),SPLIT1R=3\n OUTFIL STARTREC=2,ENDREC=4\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "main",
            "--overwrite",
            "--record-length",
            "1",
            "--control",
            "cards",
            "--out",
            "A=a",
            "--out",
            "B=b",
            "--out",
            "C=c",
        ],
    ));
    for (name, expected) in [("a", "012"), ("b", "345"), ("c", "6789"), ("main", "123")] {
        assert_eq!(fs::read(p.join(name)).unwrap(), expected.as_bytes());
    }
}
#[test]
fn outfil_csv_retains_multiline_fields_and_header_in_each_output() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(
        p.join("in"),
        "name,value,note\r\n乙,20,\"x,y\r\nz\"\r\n甲,2,\"a\"\"b\"\r\n",
    )
    .unwrap();
    fs::write(p.join("cards"),"SORT FIELDS=(2,1,NUM,A)\n OUTFIL FNAMES=BIG,INCLUDE=(2,1,NUM,GE,10)\n OUTFIL FNAMES=REST,SAVE\n").unwrap();
    ok(&run(
        p,
        &[
            "in",
            "--format",
            "csv",
            "--header",
            "--control",
            "cards",
            "--out",
            "BIG=big",
            "--out",
            "REST=rest",
        ],
    ));
    assert_eq!(
        fs::read_to_string(p.join("big")).unwrap(),
        "name,value,note\r\n乙,20,\"x,y\r\nz\"\r\n"
    );
    assert_eq!(
        fs::read_to_string(p.join("rest")).unwrap(),
        "name,value,note\r\n甲,2,\"a\"\"b\"\r\n"
    );
}
#[test]
fn outfil_processing_errors_leave_every_existing_output_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"a1b2").unwrap();
    for name in ["main", "a", "b"] {
        fs::write(p.join(name), format!("keep-{name}")).unwrap();
    }
    for cards in [
        "OPTION COPY\n OUTFIL FNAMES=A\n OUTFIL FNAMES=B,BUILD=(8,1)\n",
        "OPTION COPY\n OUTFIL FNAMES=MISSING\n",
        "OPTION COPY\n OUTFIL FNAMES=(A,B),SAMPLE=(2,3)\n",
        "OPTION COPY\n OUTFIL FNAMES=A,SAVE,INCLUDE=ALL\n",
        "OPTION COPY\n OUTFIL FNAMES=A\n OUTFIL FNAMES=A\n",
    ] {
        fs::write(p.join("cards"), cards).unwrap();
        let out = run(
            p,
            &[
                "in",
                "-o",
                "main",
                "--overwrite",
                "--record-length",
                "2",
                "--control",
                "cards",
                "--out",
                "A=a",
                "--out",
                "B=b",
            ],
        );
        assert!(!out.status.success());
        for name in ["main", "a", "b"] {
            assert_eq!(
                fs::read(p.join(name)).unwrap(),
                format!("keep-{name}").as_bytes()
            );
        }
        assert_eq!(fs::read_dir(p).unwrap().count(), 5);
    }
    let out = run(
        p,
        &[
            "in",
            "-o",
            "main",
            "--overwrite",
            "--copy",
            "--record-length",
            "2",
            "--out",
            "A=main",
        ],
    );
    assert!(!out.status.success());
}
#[test]
fn stopaft_counts_records_after_include_and_outfil_sees_post_sum() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"X1Y2X3Y4Z").unwrap();
    fs::write(
        p.join("cards"),
        "OPTION COPY,STOPAFT=2\n INCLUDE COND=(1,1,CH,EQ,C'Y')\n",
    )
    .unwrap();
    ok(&run(
        p,
        &[
            "in",
            "-o",
            "main",
            "--record-length",
            "2",
            "--control",
            "cards",
        ],
    ));
    assert_eq!(fs::read(p.join("main")).unwrap(), b"Y2Y4");
    fs::write(p.join("in"), [b'a', 1, b'b', 2, b'a', 3]).unwrap();
    fs::write(p.join("cards"),"SORT FIELDS=(1,1,CH,A)\n SUM FIELDS=(2,1,BI)\n OUTFIL FNAMES=BIG,INCLUDE=(2,1,BI,GE,3)\n OUTFIL FNAMES=REST,SAVE\n").unwrap();
    ok(&run(
        p,
        &[
            "in",
            "--record-length",
            "2",
            "--control",
            "cards",
            "--out",
            "BIG=big",
            "--out",
            "REST=rest",
        ],
    ));
    assert_eq!(fs::read(p.join("big")).unwrap(), [b'a', 4]);
    assert_eq!(fs::read(p.join("rest")).unwrap(), [b'b', 2]);
}

#[cfg(unix)]
#[test]
fn non_utf8_linux_output_paths_survive_worker_manifest_and_out_binding() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("in"), b"ba").unwrap();
    fs::write(
        p.join("cards"),
        "SORT FIELDS=(1,1,CH,A)\n OUTFIL FNAMES=A\n",
    )
    .unwrap();
    let name = OsString::from_vec(b"result-\xff".to_vec());
    let mut binding = b"A=".to_vec();
    binding.extend(b"result-\xff");
    let out = Command::new(env!("CARGO_BIN_EXE_yysort"))
        .current_dir(p)
        .args(["in", "--record-length", "1", "--control", "cards", "--out"])
        .arg(OsString::from_vec(binding))
        .env_remove("YYSORT_WORKER_DIR")
        .env_remove("YYSORT_STAGED_OUTPUT")
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(fs::read(p.join(name)).unwrap(), b"ab");
}
