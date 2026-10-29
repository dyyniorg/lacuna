// Copies address tags from an addresses.osm helper layer to the matching
// ways in the active, downloaded OSM data layer.
//
// Usage:
//  1. Open the addresses.osm file and download the cell area from OSM 
//     into another data layer.
//  2. Activate the downloaded OSM layer.
//  3. Run this script.
//
// Ways are matched by their OSM ID. Only addr:* tags are copied. Geometry,
// other tags, and `lacuna:review` are left alone. Ways that already have an
// address are skipped and reported. Only the downloaded layer is uploaded.
//
// ** NOTE: Requires the JOSM Scripting plugin with GraalJS (API v3). **

import josm from "josm"
import { DataSetUtil } from "josm/ds"

const OsmDataLayer = Java.type("org.openstreetmap.josm.gui.layer.OsmDataLayer")
const ADDRESS_KEYS = ["addr:housenumber", "addr:street", "addr:postcode", "addr:city"]
const ADDRESSES_LAYER = "addresses.osm"

// accepts addresses.osm and duplicate names such as addresses.osm (2)
function isAddressesLayer(layer) {
    const name = (layer.getName() || "").toLowerCase()
    return name === ADDRESSES_LAYER || name.startsWith(`${ADDRESSES_LAYER} (`)
}

function escapeHtml(text) {
    return String(text)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
}

function run() {
    const target = josm.layers.activeLayer

    if (!(target instanceof OsmDataLayer)) {
        josm.alert("Activate the downloaded OSM data layer first.")
        return
    }

    if (isAddressesLayer(target)) {
        josm.alert("The active layer is an addresses layer. Activate the downloaded OSM data layer instead.")
        return
    }

    const sources = []

    for (let index = 0; index < josm.layers.length; index++) {
        const layer = josm.layers.get(index)

        if (layer === target || !(layer instanceof OsmDataLayer)) {
            continue
        }

        if (isAddressesLayer(layer)) {
            sources.push(layer)
        }
    }

    if (sources.length !== 1) {
        const message = sources.length === 0
            ? "No addresses layer is open. Open the cell addresses.osm file first."
            : "More than one addresses layer is open. Close all but one."
        josm.alert(message)
        return
    }

    const source = sources[0]
    const targetUtil = new DataSetUtil(target.getDataSet())

    const updated = []
    const already = []
    const missing = []
    const incomplete = []

    targetUtil.batch(() => {
        const ways = source.getDataSet().getWays().iterator()

        while (ways.hasNext()) {
            const way = ways.next()
            const id = way.getUniqueId()

            if (id <= 0 || way.isDeleted() || !way.hasKey("addr:housenumber")) {
                continue
            }

            const match = targetUtil.way(id)

            if (!match || match.isDeleted()) {
                missing.push(id)
                continue
            }

            if (match.isIncomplete()) {
                incomplete.push(id)
                continue
            }

            if (match.hasKey("addr:housenumber")) {
                already.push(id)
                continue
            }

            for (let keyIndex = 0; keyIndex < ADDRESS_KEYS.length; keyIndex++) {
                const key = ADDRESS_KEYS[keyIndex]

                if (way.hasKey(key)) {
                    match.put(key, way.get(key))
                }
            }

            match.setModified(true)
            updated.push(match)
        }
    })

    if (updated.length > 0) {
        const ArrayList = Java.type("java.util.ArrayList")
        const selection = new ArrayList()

        for (let index = 0; index < updated.length; index++) {
            selection.add(updated[index])
        }

        target.getDataSet().setSelected(selection)
    }

    const summary = `Addresses: updated ${updated.length}, already addressed ${already.length}, incomplete ${incomplete.length}, missing ${missing.length}`
    let message = `${summary}<br>Target layer: ${escapeHtml(target.getName())}`

    if (missing.length > 0) {
        message += `<br>Missing way ids: ${missing.slice(0, 20).join(", ")}`
    }

    if (incomplete.length > 0) {
        message += `<br>Incomplete way ids: ${incomplete.slice(0, 20).join(", ")}`
    }

    josm.alert(message)
}

run()

