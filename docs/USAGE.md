# Usage

This document describes the review workflow and output layout. Primary focus is on working with [JOSM](https://josm.openstreetmap.de/), thus operability with other editors can't be guaranteed.

## Web UI

After starting the server, enter `http://localhost:3000` and select a municipality.

The web UI shows:

- Cache status for the selected municipality
- The current review round
- Per-cell OSM download URLs and file paths
- Building and address progress for each cell
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
    buildings.osm           # New building proposals for the *whole* municipality
    addresses.osm           # Address proposals for existing OSM ways for *whole* municipality
    review_state.json       # Round ID and missing building refs per cell, used by the adjust step
    progress.json           # UI: per-cell review checkmarks
    review/
      <cell>/
        buildings.osm       # New building proposals for *this* cell
        addresses.osm       # Address proposals for existing OSM ways in *this* cell
        osm_bbox.txt        # OSM download bbox and URL for JOSM for *this* cell
```

When proposals exist, the top-level `buildings.osm` and `addresses.osm` contain all proposals for the whole municipality. The files under `review/<cell>` subdirectories essentially contain the same data split into smaller areas for easier handling. The `.osm` review package files use OSM XML (supported directly in JOSM).

## JOSM workflow

1. Run the review and select a cell in the web UI.
2. Copy the OSM download URL for the cell, or use the URL in `osm_bbox.txt`. Paste the URL into JOSM to download the cell area.
3. Open the cell `buildings.osm` layer. Use `josm/lacuna-review.mapcss` to highlight objects with the `lacuna:review` helper tag.
4. In the `buildings.osm` layer, review and adjust geometries where needed. Save changes of the `buildings.osm` layer, and afterwards merge its data into the OSM layer.
5. Click "Adjust" in the web UI. This recomputes the cell `addresses.osm` from the buildings you kept and marks the building step done for the cell.
6. Open only the cell `addresses.osm` layer. Activate the downloaded OSM data layer and run `josm/apply_addresses.js`. The script migrates the address tags from the custom layer onto the OSM data layer.
7. Review the changes and upload only the downloaded OSM data layer. Remember to remove all `lacuna:*` tags before upload.
8. Mark the cell done in the web UI. This marks the address step done for the cell.

Cells with no new building proposals have no `buildings.osm` file. The web UI shows them as `Skipped`, hides the buildings file button, and lets you proceed directly to the address review. The adjust step is not needed for those cells.

### Adjust behavior

The adjust step recomputes only the cell `addresses.osm`. It reads the saved cell `buildings.osm` from disk, so save your edits first. Unsaved JOSM changes are ignored.

- The tool compares the missing-building list in `review_state.json` with the `lacuna:ref` tags in the saved `buildings.osm`. Buildings that disappeared are treated as deleted and cannot receive addresses.
- Address tags already present on kept buildings are treated as final and are not proposed again.
- Only existing OSM ways can receive new address proposals. New buildings keep the address tags written into `buildings.osm` during the review run (the first run).
- Doesn't modify `buildings.osm`.

## Overwrite behavior

Existing municipality-specific review output is not replaced unless you confirm that in the web UI or use `--overwrite` via the CLI.

