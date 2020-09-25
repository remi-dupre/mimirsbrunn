// Copyright © 2016, Canal TP and/or its affiliates. All rights reserved.
//
// This file is part of Navitia,
//     the software to build cool stuff with public transport.
//
// Hope you'll enjoy and contribute to this project,
//     powered by Canal TP (www.canaltp.fr).
// Help us simplify mobility and open public transport:
//     a non ending quest to the responsive locomotion way of traveling!
//
// LICENCE: This program is free software; you can redistribute it
// and/or modify it under the terms of the GNU Affero General Public
// License as published by the Free Software Foundation, either
// version 3 of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but
// WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
// Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public
// License along with this program. If not, see
// <http://www.gnu.org/licenses/>.
//
// Stay tuned using
// twitter @navitia
// IRC #navitia on freenode
// https://groups.google.com/d/forum/navitia
// www.navitia.io
#![allow(
    clippy::unused_unit,
    clippy::needless_return,
    clippy::never_loop,
    clippy::option_map_unit_fn
)]
use super::osm_utils::get_way_coord;
use super::OsmPbfReader;
use crate::admin_geofinder::AdminGeoFinder;
use crate::{labels, utils, Error};
use failure::ResultExt;
use osmpbfreader::StoreObjs;
use slog_scope::info;
use std::collections::{BTreeMap, HashSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use super::osm_store::{Getter, ObjWrapper};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum Kind {
    Node = 0,
    Way = 1,
    Relation = 2,
}

// TODO: make sure the ID of a way / relation doesn't change between runs (sort by admins before
//       numbering?)
pub fn streets(
    pbf: &mut OsmPbfReader,
    admins_geofinder: &AdminGeoFinder,
    db_file: &Option<PathBuf>,
    db_buffer_size: usize,
) -> Result<Vec<mimir::Street>, Error> {
    // This is the list of highway that we don't want to index
    // See [OSM Key Highway](https://wiki.openstreetmap.org/wiki/Key:highway) for background.
    const INVALID_HIGHWAY: &[&str] =
        &["bus_guideway", "escape", "bus_stop", "elevator", "platform"];

    // For the object to be a valid street, it needs to be an osm highway of a valid type,
    // or a relation of type associatedStreet.
    fn is_valid_obj(obj: &osmpbfreader::OsmObj) -> bool {
        match *obj {
            osmpbfreader::OsmObj::Way(ref way) => {
                way.tags.get("highway").map_or(false, |v| {
                    !v.is_empty() && !INVALID_HIGHWAY.iter().any(|&k| k == v)
                }) && way.tags.get("name").map_or(false, |v| !v.is_empty())
            }
            osmpbfreader::OsmObj::Relation(ref rel) => rel
                .tags
                .get("type")
                .map_or(false, |v| v == "associatedStreet"),
            _ => false,
        }
    }
    info!("reading pbf...");
    let mut objs_map = ObjWrapper::new(db_file, db_buffer_size)?;
    pbf.get_objs_and_deps_store(is_valid_obj, &mut objs_map)
        .context("Error occurred when reading pbf")?;
    info!("reading pbf done.");

    // Builder for street object
    let build_poi =
        |id: String, name: String, coord: mimir::Coord, admins: Vec<Arc<mimir::Admin>>| {
            let admins_iter = admins.iter().map(Deref::deref);
            let country_codes = utils::find_country_codes(admins_iter.clone());
            mimir::Street {
                id,
                label: labels::format_street_label(&name, admins_iter, &country_codes),
                name,
                weight: 0.,
                zip_codes: utils::get_zip_codes_from_admins(&admins),
                administrative_regions: admins,
                coord,
                approx_coord: Some(coord.into()),
                distance: None,
                country_codes,
                context: None,
            }
        };

    // List of outputed streets
    let mut street_list = Vec::new();

    // Sometimes, streets can be divided into several "way"s that still have the same street name.
    // The reason why a street is divided may be that a part of the street become
    // a bridge/tunnel/etc. In this case, a "relation" tagged with (type = associatedStreet) is used
    // to group all these "way"s. In order not to have duplicates in autocompletion, we should tag
    // the osm ways in the relation not to index them twice.
    let mut street_in_relation = HashSet::new();

    objs_map.for_each_filter(Kind::Relation, |obj| {
        let rel = obj.relation().expect("invalid relation filter");
        let rel_name = rel.tags.get("name").map(String::as_str);
        let first_street = rel
            .refs
            .iter()
            .filter(|ref_obj| ref_obj.member.is_way() && ref_obj.role == "street")
            .filter_map(|ref_obj| {
                let obj = objs_map.get(&ref_obj.member)?;
                let way = obj.way()?;
                let coord = get_way_coord(&objs_map, &way);
                let name = rel_name
                    .or_else(|| way.tags.get("name").map(String::as_str))
                    .unwrap_or("");

                // Add osmid of all the relation members in the set
                // We don't create any street for all the osmid present in street_rel
                street_in_relation.insert(ref_obj.member);

                Some(
                    get_street_admin(admins_geofinder, &objs_map, &way)
                        .into_iter()
                        .enumerate()
                        .map(|(i, admins)| {
                            build_poi(
                                format!("street:osm:relation{}-{}", rel.id.0, i),
                                name.to_string(),
                                coord,
                                admins,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .next();

        if let Some(street) = first_street {
            street_list.extend(street.into_iter());
        }
    });

    // We merge all the ways with same `way_name` and `admin list of level(=city_level)`
    // We use a Map to keep track of the way of smallest Id for a given pair of "name + cities list"
    let mut name_admin_map = BTreeMap::new();

    objs_map.for_each_filter(Kind::Way, |obj| {
        let osmid = obj.id();
        let way = obj.way().expect("invalid way filter");

        if street_in_relation.contains(&osmid) {
            return;
        }

        if let Some(name) = way.tags.get("name") {
            for admins in get_street_admin(admins_geofinder, &objs_map, way) {
                let cities: Vec<_> = admins.into_iter().filter(|admin| admin.is_city()).collect();
                name_admin_map
                    .entry((name.to_string(), cities))
                    .and_modify(|stored| *stored = std::cmp::min(*stored, osmid))
                    .or_insert(osmid);
            }
        }
    });

    // Create a street for each way with osmid present in objs_map
    street_list.extend(
        name_admin_map
            .values()
            .filter_map(|min_id| {
                let obj = objs_map.get(&min_id)?;
                let way = obj.way()?.clone();
                let coord = get_way_coord(&objs_map, &way);
                let name = way.tags.get("name")?.to_string();
                Some(
                    get_street_admin(admins_geofinder, &objs_map, &way)
                        .into_iter()
                        .enumerate()
                        .map(move |(i, admins)| {
                            build_poi(
                                format!("street:osm:way:{}-{}", way.id.0, i),
                                name.to_string(),
                                coord,
                                admins,
                            )
                        }),
                )
            })
            .flatten(),
    );

    Ok(street_list)
}

fn get_street_admin<T: StoreObjs + Getter>(
    admins_geofinder: &AdminGeoFinder,
    obj_map: &T,
    way: &osmpbfreader::objects::Way,
) -> Vec<Vec<Arc<mimir::Admin>>> {
    /*
        To avoid corner cases where the ends of the way are near
        administrative boundaries, the geofinder is called
        on a middle node.
    */
    let nb_nodes = way.nodes.len();
    way.nodes
        .iter()
        .skip(nb_nodes / 2)
        .filter_map(|node_id| obj_map.get(&(*node_id).into()))
        .filter_map(|node_obj| {
            node_obj.node().map(|node| geo_types::Coordinate {
                x: node.lon(),
                y: node.lat(),
            })
        })
        .next()
        .map_or(vec![], |c| admins_geofinder.get_all(&c))
}

pub fn compute_street_weight(streets: &mut Vec<mimir::Street>) {
    for st in streets {
        for admin in &mut st.administrative_regions {
            if admin.is_city() {
                st.weight = admin.weight;
                break;
            }
        }
    }
}
