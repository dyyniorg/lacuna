//! Address candidates, normalization, and address assignment.

use std::collections::HashSet;

use serde_json::{Value, json};

pub(super) struct AddressInfo {
    pub(super) housenumber: String,
    pub(super) street: String,
    pub(super) postcode: String,
    pub(super) city: String,
    pub(super) full: String,
    pub(super) point: (f64, f64),
}

// An OSM building that exists but has no address tags, with the official
// address that should be reviewed onto it.
pub struct AddressFix {
    pub way_id: String,
    pub housenumber: String,
    pub street: String,
    pub postcode: String,
    pub city: String,
    // Official building whose address this is.
    pub owner: Option<String>,
}

// The housenumber as OSM writes it, e.g. "4a" from parts "4" and "a".
pub(super) fn official_housenumber(properties: &Value) -> Option<String> {
    let base = properties.get("number_part_of_address_number").and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|number| number.to_string()))
    })?;

    let mut number = base;

    if let Some(letter) = properties
        .get("subdivision_letter_of_address_number")
        .and_then(Value::as_str)
    {
        number.push_str(letter);
    }

    if let Some(letter) = properties
        .get("subdivision_letter_of_address_number2")
        .and_then(Value::as_str)
    {
        number.push_str(letter);
    }

    Some(number)
}

// Key of an address, normalized number and street.
pub fn address_key(housenumber: &str, street: &str) -> String {
    format!("{} {}", normalize(housenumber), normalize(street))
}

// Lowercase alphanumerics only, for comparing address values.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

// VTJ stores postal office names in uppercase, OSM uses normal case.
pub(super) fn title_case(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut new_word = true;

    for character in text.chars() {
        if character.is_alphabetic() {
            if new_word {
                result.extend(character.to_uppercase());
                new_word = false;
            } else {
                result.extend(character.to_lowercase());
            }
        } else {
            new_word = true;
            result.push(character);
        }
    }

    result
}

// One address assignment candidate: an existing OSM building or a
// missing official building that will be added from buildings.osm.
pub(super) enum CandKind {
    Osm { way_id: String, street: Option<String> },
    New { index: usize },
}

pub(super) struct Cand {
    pub(super) kind: CandKind,
    pub(super) area: f64,
    pub(super) key: String,
    pub(super) housenumber: Option<String>,
}

#[derive(Default)]
pub(super) struct AddressCounts {
    pub(super) none: usize,
    pub(super) matched: usize,
    pub(super) changed: usize,
    pub(super) missing: usize,
    pub(super) multiple: usize,
    pub(super) duplicate: usize,
}

// Address assignment results shared by all parcels in one comparison.
#[derive(Default)]
pub(super) struct AddressState {
    pub(super) fixes: Vec<AddressFix>,
    pub(super) counts: AddressCounts,
    attached: HashSet<String>,
    attached_ways: HashSet<String>,
}

pub(super) fn set_address_state(result_features: &mut [Value], index: usize, state: &str) {
    if let Some(properties) = result_features[index]["properties"].as_object_mut() {
        properties.insert("address_state".to_string(), json!(state));
    }
}

fn set_address_with_info(result_features: &mut [Value], index: usize, state: &str, street_equal: Option<bool>) {
    if let Some(properties) = result_features[index]["properties"].as_object_mut() {
        properties.insert("address_state".to_string(), json!(state));
        properties.insert("address_street_equal".to_string(), json!(street_equal));
    }
}

// Assigns the official addresses of one parcel (or one loose building) to
// candidates: an address already present wins, otherwise the largest
// candidate without address information receives it.
pub(super) fn assign_addresses(
    result_features: &mut [Value],
    addresses: &[(usize, &AddressInfo)],
    candidates: Vec<Cand>,
    state: &mut AddressState,
    placed: &HashSet<String>,
    existing_only: bool,
    permanent_ids: &[Option<String>],
) {
    let mut used = vec![false; candidates.len()];
    for (owner, address) in addresses {
        // NOTE: one address string must not be attached twice, several official
        // records can share one address
        let address_key = address_key(&address.housenumber, &address.street);

        // check: a kept building already carries this address (if yes, leave it alone)
        if placed.contains(&address_key) {
            state.counts.matched += 1;
            set_address_with_info(result_features, *owner, "matched", None);

            continue;
        }

        // NOTE: matching does not consume a candidate: several official records
        // can share one address, and the address only needs to exist once
        let match_candidate = candidates.iter().find(|candidate| {
            candidate
                .housenumber
                .as_ref()
                .is_some_and(|number| normalize(number) == normalize(&address.housenumber))
        });

        if let Some(candidate) = match_candidate {
            let street_equal = match &candidate.kind {
                CandKind::Osm { street, .. } => street
                    .as_ref()
                    .map(|street| normalize(street) == normalize(&address.street)),
                CandKind::New { .. } => None,
            };

            state.counts.matched += 1;
            set_address_with_info(result_features, *owner, "matched", street_equal);

            continue;
        }

        // check: the same address already being attached for another record
        if state.attached.contains(&address_key) {
            state.counts.duplicate += 1;
            set_address_with_info(result_features, *owner, "duplicate", None);

            continue;
        }
        let best = candidates
            .iter()
            .enumerate()
            .filter(|(index, candidate)| {
                !used[*index]
                    && candidate.housenumber.is_none()
                    && match &candidate.kind {
                        CandKind::Osm { way_id, .. } => !state.attached_ways.contains(way_id),
                        CandKind::New { .. } => !existing_only,
                    }
            })
            .max_by(|(_, left), (_, right)| left.area.total_cmp(&right.area).then_with(|| right.key.cmp(&left.key)))
            .map(|(index, _)| index);

        match best {
            Some(index) => {
                used[index] = true;

                match &candidates[index].kind {
                    CandKind::Osm { way_id, .. } => {
                        state.attached_ways.insert(way_id.clone());
                        state.fixes.push(AddressFix {
                            way_id: way_id.clone(),
                            housenumber: address.housenumber.clone(),
                            street: address.street.clone(),
                            postcode: address.postcode.clone(),
                            city: address.city.clone(),
                            owner: permanent_ids[*owner].clone(),
                        });
                    }
                    CandKind::New { index } => {
                        let properties = result_features[*index]["properties"]
                            .as_object_mut()
                            .expect("missing properties");

                        if let Some(suggested) = properties.get_mut("suggested_tags").and_then(Value::as_object_mut) {
                            for (key, value) in [
                                ("addr:housenumber", &address.housenumber),
                                ("addr:street", &address.street),
                                ("addr:postcode", &address.postcode),
                                ("addr:city", &address.city),
                            ] {
                                if !value.is_empty() {
                                    suggested.insert(key.to_string(), json!(value));
                                }
                            }
                        }
                    }
                }

                state.attached.insert(address_key);

                state.counts.missing += 1;
                set_address_with_info(result_features, *owner, "missing", None);
            }
            None => {
                let any_addressed = candidates.iter().any(|candidate| candidate.housenumber.is_some());

                if any_addressed {
                    state.counts.changed += 1;
                    set_address_with_info(result_features, *owner, "changed", None);
                } else {
                    state.counts.missing += 1;
                    set_address_with_info(result_features, *owner, "missing", None);
                }
            }
        }
    }
}
