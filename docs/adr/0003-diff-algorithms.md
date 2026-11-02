# 0003 — Diff and compare algorithms

Date: 2026-11-02 · Status: accepted

## Text
1. Decode (BOM, UTF-16 without BOM, chardetng fallback); binary if NUL in first 8 KB.
2. Normalize each line for *matching only* (whitespace, case, line endings).
3. Intern lines and run imara-diff **Myers** (near-linear thanks to git's
   heuristics). If `lines × hunks` is under a budget (~50 ms of work), recompute
   with **Histogram**, which gives more readable hunks but degrades to
   `O(lines × hunks)`: 12.6 s vs 26 ms on 1M lines with 10k scattered edits,
   found by the performance budget tests. Then run the indent-aware slider
   post-processing so hunks land where humans expect. Byte-identical texts
   skip diffing entirely.
4. Inside each changed block, pair lines by bigram (Dice) similarity with an
   order-preserving weighted LCS, capped at 40k cells; larger blocks pair
   positionally. Result: one flat `Vec<Row>` shared by both panes, scrolling,
   the ruler and navigation.
5. Word-level highlights are computed lazily per visible row (Myers on word
   tokens), honoring the ignore rules.

## Folders
1. One task per directory pair on a dedicated rayon pool (thread count is a
   setting: lower it for HDDs and network shares).
2. Merge-join over sorted keys; keys are NFC-normalized and case-folded where
   the platform is case-insensitive. Each side keeps its own spelling.
3. Tiered file verdicts: size differs → different; smart mode trusts equal
   size + mtime; otherwise stream both files and stop at the first differing
   256 KiB chunk. Every verdict records its `basis`.
4. Errors are per entry. The tree is an arena with contiguous children.

## Not yet
Hashing (BLAKE3/xxh3) is reserved for remote sides, move detection and a
persistent cache; for two local files, streaming comparison is cheaper.
