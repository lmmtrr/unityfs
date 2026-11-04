# unityfs

A pure Rust library for parsing, reading, and extracting assets from Unity serialized files and AssetBundles.

## Usage

Add `unityfs` to your `Cargo.toml`:

```toml
[dependencies]
unityfs = "0.1"
```

Then you can read asset bundles and extract assets programmatically:

```rust
use unityfs::{is_unity_bundle, extract_unity_assets};

let bundle_bytes = std::fs::read("my_model.ab").unwrap();
if is_unity_bundle(&bundle_bytes) {
    extract_unity_assets(&bundle_bytes, "extracted_output/").unwrap();
}
```

Raw SerializedFiles such as `sharedassets0.assets` or `level0` are also supported. When opened from a path, sibling `.resS` and `.resource` files are loaded automatically:

```rust
use unityfs::{is_unity_file, extract_unity_assets_from_path};

extract_unity_assets_from_path("Game_Data/sharedassets0.assets", "extracted_output/").unwrap();
```

When working with bytes, pass the stream files explicitly:

```rust
use unityfs::extract_unity_assets_with_resources;

let data = std::fs::read("sharedassets0.assets").unwrap();
let ress = std::fs::read("sharedassets0.assets.resS").unwrap();
extract_unity_assets_with_resources(&data, "sharedassets0.assets", vec![("sharedassets0.assets.resS".to_string(), ress)], "extracted_output/").unwrap();
```

## Acknowledgement

- [UnityPy](https://github.com/K0lb3/UnityPy)

## License

[MIT License](https://github.com/lmmtrr/unityfs/blob/main/LICENSE)
