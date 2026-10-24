# YYSortの開発・内部仕様

[文書全体](../README.md) / [処理パイプライン](architecture.md)

## ビルド・検査

リポジトリルートでRust stable、WindowsではMSVCビルド環境を使用します。
YYLibraryの固定コミットはCargoが取得します。Cargo.tomlの`rev`とCargo.lockを同時に管理してください。

```text
cargo build --release --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --test-threads=1
```

文字コード本体とその試験はYYLibraryで管理します。利用側の全件比較・OS資源上限の試験は本リポジトリで行います。
CLIの利用説明は[同梱ヘルプ](../../help/help.md)へ記載します。

## 性能検査と配布

Python 3.10以降を利用します。生成先のドライブの空き容量を確認してください。

```text
python demo/benchmark.py --exe target/release/yysort --mib 256 --memory 128MiB --threads 2 --directory target/benchmark-new
python demo/benchmark.py --exe target/release/yysort --mib 256 --memory 128MiB --threads 2 --directory target/benchmark-outfil-new --outfil --reformat
python tools/package.py --test
```

[専用CI](../../.github/workflows/yysort.yml)はWindows/Linuxで検査・全件照合・release・配布を実施します。
`dist/yysort/`へCLI、利用者文書、開発者文書、履歴、作例、ソース、ライセンスを出力します。
採用したYYLibraryのソースも`source/vendor/YYLibrary`へ保存し、Cargo.tomlとCargo.lockの固定Git参照を維持します。
日付・環境付きの測定値は[履歴](../history/README.md)に置きます。
