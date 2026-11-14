# Usage

This document describes the review workflow and output layout. Primary focus is on working with [JOSM](https://josm.openstreetmap.de/), thus operability with other editors can't be guaranteed.

## Web UI

After starting the server, enter `http://localhost:3000` and select a municipality.

The web UI shows:

- Cache status for the selected municipality
- The current review round
- Per-cell OSM download URLs and file paths
- Address progress for each cell
- Overwrite confirmation

## Output layout

With the default cache directory:

```text
data/
  finland_latest.osm.pbf    # CACHE: OSM extract
  <municipality-slug>/
    ryhti_buildings.json    # CACHE: building points and properties
    ryhti_addresses.json    # CACHE: address records
    nls_footprints.json     # CACHE: building footprint polygons
    parcels.json            # CACHE: land parcels
    osm_buildings.json      # CACHE: OSM building ways extracted from the PBF
    addresses.osm           # Address proposals for existing OSM ways for *whole* municipality
    review_state.json       # Round ID
    progress.json           # UI: per-cell review checkmarks
    review/
      <cell>/
        addresses.osm       # Address proposals for existing OSM ways in *this* cell
        osm_bbox.txt        # OSM download bbox and URL for JOSM for *this* cell
```

When address proposals exist, the top-level `addresses.osm` contains all proposals for the whole municipality. The files under `review/<cell>` subdirectories contain the same data split into smaller areas for easier handling. The `.osm` review package files use OSM XML (supported directly in JOSM).

Review cells use a fixed national grid in WGS84 (EPSG:4326) with origin 17.0E, 57.0N and cell size 0.12 by 0.06 degrees. The grid generation is deterministic, and produces cell IDs of form `{row:03}_{col:03}`. Cells without any official building points are not included in the review process. Cells without address proposals are not written.

## JOSM workflow

1. Run the review and select a cell in the web UI.
2. Copy the OSM download URL for the cell, or use the URL in `osm_bbox.txt`. Paste the URL into JOSM to download the cell area.
3. Open the cell `addresses.osm` layer. Use `josm/lacuna-review.mapcss` to highlight objects with the `lacuna:review` helper tag.
4. Review the address proposals in the `addresses.osm` layer.
5. Activate the downloaded OSM data layer and run `josm/apply_addresses.js`. The script copies the address tags from the review layer onto the matching OSM ways.
6. Review the changes and upload only the downloaded OSM data layer. Remember to remove all remaining `lacuna:*` tags before upload.
7. Mark the cell done in the web UI.

Only existing OSM ways receive address proposals. The tool does not propose new building geometries.

## Overwrite behavior

Existing municipality-specific review output is not replaced unless you confirm that in the web UI or use `--overwrite` via the CLI. Using `--overwrite` also resets saved cell progress. Confirming overwrite in the web UI replaces review output but keeps progress.

