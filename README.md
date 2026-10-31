# Lacuna

Local tool for importing Finnish building and address data semi-automatically into OpenStreetMap. It handles the spatial comparison of official building and address data with OSM extracts, writes per-cell review packages, and helps the user through the changeset review process.

Official data sources (CC BY 4.0):

- [National Land Survey of Finland (NLS)](https://www.maanmittauslaitos.fi/en): land parcels and Topographic database building polygons via INSPIRE WFS
- [Finnish Environment Institute (Syke)](https://www.syke.fi/en): Ryhti buildings and building addresses

> [!NOTE]
> Even though the tool is capable of processing large amounts of data into OSM, it should not be used for [fully automated imports](https://wiki.openstreetmap.org/wiki/Import). The user must always review the changesets before uploading them.

## What it does

For a given municipality, the workflow is:

1. Download and cache Ryhti buildings and addresses, NLS building footprints, land parcels, and the OSM extract.
2. Merge Ryhti building points with NLS building footprints. Footprints that do not match a current Ryhti point are discarded.
3. Compare the merged building data with existing OSM buildings from the PBF extract by geometry, using an R-tree.
4. Assign official addresses to existing OSM buildings and to missing building candidates from the comparison.
5. Split the result into grid cells of approximately 2x2 km.

See [`docs/PIPELINE.md`](docs/PIPELINE.md) for a detailed description of the matching process.

## Usage

The binary can be downloaded as a pre-compiled [release](https://github.com/dyyniorg/lacuna/releases/latest) or built manually with `cargo build --release`.

The tool can be used via the CLI or the local web UI, which is the default. Running `lacuna` without arguments starts the web UI at `http://localhost:3000`. Use `--listen-all` to listen on `0.0.0.0:3000` instead.

See [`docs/USAGE.md`](docs/USAGE.md) for a detailed description of the full workflow with the JOSM editor.

### CLI modes

CLI modes require `--municipality <code>`. A complete list of municipality codes is available at [stat.fi](https://stat.fi/en/luokitukset/kunta/kunta_1_20260101).

```sh
lacuna --municipality <code> --review
lacuna --municipality <code> --fetch
lacuna --municipality <code> --adjust <cell>
```

Use only one of `--review`, `--fetch`, or `--adjust` at a time.

## Configuration

| Option | Default | Description |
| - | - | - |
| `--cache <dir>` | `data` | Cache and review root |
| `--cache-max-age <days>` | `7` | Maximum age of a cached resource. Use `0` to force a refresh |
| `--listen-all` | off | Listen on `0.0.0.0:3000` instead of `localhost:3000` |
| `--municipality <code>` | none | Municipality code for CLI modes |
| `--overwrite` | off | Replace existing review output without confirmation |

Without `--overwrite`, the CLI review mode stops when review output exists. The web UI asks for confirmation instead.
