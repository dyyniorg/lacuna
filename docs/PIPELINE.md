# Matching process

The tool matches current Finnish official building and address data against OSM data.

| Input | Use |
| - | - |
| Ryhti building points | Defines the current official buildings |
| Ryhti address records | Links addresses to official buildings |
| NLS building footprints | Provides polygon geometry for official buildings |
| NLS land parcels | Limits address assignment to the same property |
| Geofabrik Finland PBF extract | Provides existing OSM building ways and address tags |

The matching has three stages:

1. Join official building points to NLS footprints.
2. Compare official building polygons with OSM building polygons.
3. Assign official addresses to the resulting candidates.

## Scope

The tool calculates a bounding box around the official building points and expands it by 0.02 degrees on every side. NLS footprints, land parcels, and OSM nodes are read only from this expanded area.

## Official buildings

A building enters matching only when it has point geometry, does not have a `demolition_date` string, and its `usage_status` does not end in `06`, `07`, or `08`. Records without a JSON properties object are skipped.

The matching uses these properties:

- `permanent_building_identifier`
- `building_key`
- `property_identifier`

An official building without a usable NLS footprint is excluded. NLS footprints are never matched as standalone buildings.

## NLS footprints

NLS footprints are XML polygon features. Each `posList` ring (i.e. a closed sequence of points forming the polygon) is processed as follows:

- The source order is latitude first, longitude second. The tool converts it to longitude first.
- A ring must have at least 4 positions.
- The first and last position must be equal. Open rings are ignored.
- Only outer rings are used. Interior rings are ignored for matching.

A Polygon produces one footprint entry. A MultiPolygon produces one entry per outer ring.

## OSM buildings

An OSM building way enters matching when:

- It has a `building` tag, with any value.
- All its node references are inside the municipality bounding box.
- Its ring is closed.
- Its ring has at least 4 nodes.

A way with a missing node is skipped. Only outer rings are used. OSM building relations are not compared; only OSM ways are read.

Each OSM candidate keeps its way ID, tags, outer ring, polygon area, and bounding box. Building comparison uses geometry only, not tags.

## Land parcels

Each land parcel keeps its `nationalLandReference`, outer polygon parts, total area, and bounding box. The `nationalLandReference` is matched against the official building `property_identifier`.

## Official addresses

Address records are grouped by `building_key`.

| Field | Source |
| - | - |
| Housenumber | `number_part_of_address_number`, plus `subdivision_letter_of_address_number` and `subdivision_letter_of_address_number2` |
| Street | `address_name_fin` |
| Postcode | `postal_code` |
| City | `postal_office_fin`, converted to title case |
| Full address | `address_fin` |
| Point | Address feature geometry |

The housenumber concatenates the base number and subdivision letters. For example, `4`, `a`, and `b` become `4ab`.

An address record without a building key, housenumber, or point is ignored.

Address comparison uses lowercase alphanumeric characters only:

```text
<normalized housenumber> <normalized street>
```

## Joining official points to NLS footprints

Each official building point is joined to an NLS footprint within 20 meters.

For each point:

1. Find footprints whose bounding boxes intersect a 20 meter box around the point.
2. If the point is inside a footprint, keep the footprint with the smallest area.
3. Otherwise, calculate the distance to the footprint ring.
4. Keep the nearest footprint if the distance is at most 20 meters.

This produces one provisional claim per official building.

Footprint ownership is resolved one to one:

- An inside claim beats a near claim.
- If both claims have the same inside/near status, the shorter distance wins.
- If a tie remains, the earlier official building in the input wins.

Each footprint belongs to one official building. Other claimants are treated as having no footprint.

The official building geometry becomes the claimed NLS footprint geometry.

## Comparing official buildings with OSM

Each joined official footprint is compared with OSM polygons whose bounding boxes intersect it.

For each candidate:

1. Calculate the intersection area.
2. Divide it by the smaller polygon area.
3. Keep the largest ratio.

If a polygon operation fails, the tool falls back to bounding-box overlap. A zero-area polygon produces a ratio of zero.

| Ratio | State | Meaning |
| - | - | - |
| `>= 0.7` | `matched` | The OSM building covers at least 70 percent of the smaller polygon |
| `> 0` and `< 0.7` | `changed` | The OSM building overlaps, but its shape or position differs |
| `0` | `missing` | No OSM building overlaps the official footprint |

## Assigning addresses

Address assignment is done per land parcel, with a fallback for buildings without a parcel identifier.

### Grouping by parcel

Official buildings with a `property_identifier` are grouped by that value. Buildings without a `property_identifier` use the loose-building fallback.

### Assigning OSM buildings to parcels

Each OSM building is assigned to one parcel:

1. Calculate the center of its bounding box.
2. Find parcels whose polygon contains the center.
3. If several parcels contain the center, choose the smallest by area, then by parcel key.
4. If no parcel contains the center, choose the parcel with the largest overlap area.
5. A fallback parcel with zero overlap is not used.

Parcel assignment only limits address candidates. It does not change geometry.

### Selecting official addresses

For each official building in a parcel:

- No address record: state `no_address`.
- More than one address record: state `multiple`; the address is skipped.
- Exactly one address record: the address enters assignment.

The same rule applies to loose buildings.

### Address candidates

For each parcel, address candidates are:

- OSM buildings assigned to the parcel by center or overlap.
- OSM buildings linked by the building match, even when their center is in another parcel.
- New candidates from official buildings whose state is `missing`.

A new candidate is excluded when:

- It has the `multiple` address state.
- Its official ID is in the deleted-building override list.
- It has no usable footprint area.

During adjustment, only existing OSM buildings receive addresses.

Duplicate OSM way IDs are removed.

### Assignment order

Addresses are sorted by their full address string.

For each address:

1. If the address key is already marked as placed, state `matched`.
2. If a candidate has the same normalized housenumber, state `matched`.
3. If the address key is already attached to another official building, state `duplicate`.
4. Otherwise, choose the largest candidate that:
   - has no housenumber,
   - has not been used in this assignment,
   - is not an OSM way that already received an address,
   - is not a new building when only existing OSM buildings are allowed.
5. If areas are equal, the smaller candidate key wins.

If no candidate is available:

- State `changed` when a candidate already has an address.
- State `missing` when no candidate has an address.

### Address states

| State | Meaning |
| - | - |
| `matched` | The address already exists on a candidate, or was already placed |
| `duplicate` | The same address was already assigned to another official building |
| `missing` | The address was assigned to a building without an address, or no candidate exists |
| `changed` | Candidates exist, but they already have different addresses |
| `multiple` | The official building has more than one address record |
| `no_address` | The official building has no address record |

### Loose buildings

Buildings without a `property_identifier` do not use parcels.

For a loose building with exactly one address:

1. Find OSM polygons within 15 meters of the address point.
2. A polygon containing the point has distance zero.
3. Otherwise, use the distance to the polygon ring.
4. Keep candidates within 15 meters.
5. Use the same assignment rules.

Loose buildings can receive addresses only on existing OSM ways.

## Matching constraints

- Only outer rings are used. Building holes are ignored.
- OSM building relations are not compared.
- OSM ways that extend outside the municipality bounding box are skipped.
- Building states use geometry only.
- Official buildings without a usable NLS footprint are excluded.
- Buildings with multiple official addresses are skipped.
- A shared address is attached only once.
