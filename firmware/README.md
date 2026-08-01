# Official firmware inputs

`micropython-v1.28.0.toml` pins the exact upstream artifacts used by Renvo Emulator's
four-board qualification baseline. The manifest is committed; downloaded
firmware is content-addressed evidence under `.remu/` and is intentionally not
committed.

Fetch through the pinned Docker boundary and verify all hashes:

```sh
scripts/fetch-micropython.sh
```

The final qualification gate executes the primary artifacts unchanged. Symbolic
development builds and companion images may aid diagnosis, but cannot replace
these bytes as acceptance evidence.
