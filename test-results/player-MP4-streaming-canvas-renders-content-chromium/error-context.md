# Page snapshot

```yaml
- generic [ref=e1]:
  - heading "HEVC Video Player" [level=1] [ref=e2]
  - generic [ref=e3] [cursor=pointer]: Drop an HEVC .mp4/.mkv file here or click to browse
  - generic [ref=e4]:
    - button "MP4" [active] [ref=e5] [cursor=pointer]
    - button "MKV (with subs)" [ref=e6] [cursor=pointer]
  - generic [ref=e8]:
    - generic [ref=e9]: "Video: 1920x1080, 120.2s — Ready"
    - slider [ref=e12] [cursor=pointer]: "17"
    - generic [ref=e13]:
      - button "Play" [ref=e14] [cursor=pointer]
      - button "Pause" [ref=e15] [cursor=pointer]
      - button "Restart" [ref=e16] [cursor=pointer]
      - generic [ref=e17]: 0:01 / 2:00
      - combobox [ref=e18]:
        - option "0.25x"
        - option "0.5x"
        - option "1x" [selected]
        - option "1.5x"
        - option "2x"
      - generic [ref=e19]: 25 fps
```