# unityfs

A pure Rust library for parsing, reading, and extracting assets from Unity serialized files and AssetBundles.

## Usage

Add `unityfs` to your `Cargo.toml`:

```toml
[dependencies]
unityfs = "0.1.1"
```

Then you can read asset bundles and extract assets programmatically:

```rust
use unityfs::{is_unity_bundle, extract_unity_assets};

let bundle_bytes = std::fs::read("my_model.ab").unwrap();
if is_unity_bundle(&bundle_bytes) {
    extract_unity_assets(&bundle_bytes, "extracted_output/").unwrap();
}
```

## Acknowledgement

- [UnityPy](https://github.com/K0lb3/UnityPy)

## License

[MIT License](https://github.com/lmmtrr/unityfs/blob/main/LICENSE)
