# YYSort

Windows/Linux用CLI外部ソートです。巨大な固定長ファイルとRFC形式CSVを、メモリ上限を指定して処理します。

- 安定SORT、MERGE、COPY、複数キー、抽出・集計・重複排除。
- INCLUDE/OMIT、INREC/OUTREC BUILD/FIELDS/OVERLAY、OUTFILの抽出・変形・分割。
- 日本語・Unicode・EBCDIC・外部対応表。文字コード処理は[YYLibrary](https://github.com/ykwyuta/YYLibrary)を利用。
- CPUアフィニティ・メモリ・累積CPU時間・一時領域の制限と使用量表示。

**IBM DFSORTの全機能互換ではありません。** 対応する構文・データ形式・上限は[同梱ヘルプ](help/help.md)で確認してください。

## すぐ使う

```text
yysort input.dat -o sorted.dat --record-length 80 --key 1:8:CH:A --memory 512MiB --threads 2 --stats
yysort input.csv -o sorted.csv --format csv --header --key 3:NUM:D --encoding CP932
yysort --manual
```

`--memory`はLinuxの仮想アドレス空間／Windowsのコミットメモリ上限で、RSS上限ではありません。
既存出力の保全・取消・終了コードもヘルプを参照してください。

## 文書

- [利用者向け：設定・コマンド・作例](docs/user/README.md)
- [開発者向け：内部仕様・ビルド・検証・配布](docs/developer/README.md)
- [検討経緯・作業記録：提案、測定、検証履歴](docs/history/README.md)

[GPL-3.0-or-later](COPYING)と[追加許可](COPYING.EXCEPTION)。第三者素材の条件は[NOTICE](NOTICE.md)を参照してください。
