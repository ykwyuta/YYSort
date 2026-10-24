# yysortの検証

sample.csvはヘッダー、カンマ、引用符内の改行、二重引用符を含むUTF-8です。
固定長サンプルは12バイトで、sort.ctlを使ってキー順に並べ、重複キーの先頭を残します。

```text
yysort demo/sample.csv -o sorted.csv --format csv --header --key 1:TEXT:A --sum 2:NUM
yysort demo/sample.fixed -o sorted.fixed --record-length 12 --control demo/sort.ctl
```

巨大ファイルはリポジトリに含めずbenchmark.pyで生成します。
空の専用ディレクトリを指定してください。既存データは上書きしません。
親Pythonも全ファイルをメモリに載せず逐次検証します。入力・出力に加え、ランと出力ステージング用の空き容量が必要です。

OUTFILの例（名前付き2出力）:

```text
yysort demo/sample.fixed --record-length 12 --control demo/outfil.ctl --out FIRST=first.fixed --out REST=rest.fixed
```

性能検証に `--outfil` を付けると、主出力と巡回分割2出力を同時に生成し、分割結果の全レコードも照合します。

項目間比較・OVERLAY・出力列指定の作例：

```text
yysort demo/sample.fixed -o overlay.fixed --record-length 12 --control demo/overlay.ctl
```

元の12バイトの末尾2バイトをOKにし、末尾に複写したキーでソートします。OUTRECでキー複写部分を除き、
14バイト目へEND、末尾へ空白を置いた17バイトのレコードにします。重複は保持します。
性能検証の`--reformat`は、項目間比較とINREC/OUTREC OVERLAYを適用し、変形後の全バイトを照合します。
