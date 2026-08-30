use std::{
    collections::{BTreeSet, HashMap, HashSet},
    pin::Pin,
    sync::{Arc, Weak},
    time::Instant,
};

use dashmap::{DashMap, DashSet};
use pumpkin_util::math::{
    block_box::BlockBox,
    bounding_box::BoundingBox,
    vector3::{Vector3, packed_chunk_pos},
};
use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelBridge,
    ParallelIterator,
};
use tokio::{
    pin,
    sync::{
        Mutex, Notify, RwLock,
        mpsc::{Receiver, Sender, error::TrySendError},
        oneshot,
    },
};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::entity::EntityBase;

/*
the ways we can keep this in sync
modify cache when entities are modified, requires thread safety via RwLock or Mutex or some other method
a write queue like we have now, this requires a point in time where syncing is done though
    either on the tick loop for a world, such as after entities, block entities and others have run their tick method
    or on a separate thread, with this seperate thread idea (tokio::spawn)
        we can either make it a singlular point, like how the tick loop did it, such as doing it every X milliseconds based on current tick rate.
        or make it near continous where we take incoming cache ops and act on them as we get them, this would be similar to doing them when entities change but slightly detached to not slow down entity ticks

*/

async fn actor(mut rx: Receiver<CacheOp>) {
    let mut tracking_section_map: HashMap<i64, EntityCacheChunk> = HashMap::new();
    let mut chunk_index_tree: BTreeSet<i64> = BTreeSet::new();
    let mut dirty_subchunk_indexs: BTreeSet<i64> = BTreeSet::new();
    let mut i = 0;
    let id = rand::random_range(0..=1000);
    loop {
        i += 1;
        let Some(cache_op) = rx.recv().await else {
            break;
        };
        // info!("{} - starting op {:?} - {}",id,&cache_op,i);
        // match cache_op {
        //     CacheOp::CleanCache { start, is_done_callback } => {is_done_callback.notify_one();}
        //     _ => {}
        // };
        // continue;
        match cache_op {
            CacheOp::Add { entity } => {
                // continue;
                // let entity: Option<Arc<dyn EntityBase>> = entity.upgrade();
                // if entity.is_none() {
                //     continue
                // }
                // let entity = entity.expect("msg");

                let Some(entity): Option<Arc<dyn EntityBase>> = entity.upgrade() else {
                    info!("A - lost ref");
                    continue;
                };
                if Uuid::parse_str("6280580c-8dc8-41c9-b62f-6456c1ec5bc7").unwrap()
                    == entity.get_entity().entity_uuid
                {
                    info!("added player to cache");
                }
                // info!(
                //     "add entity - {:#?}",
                //     entity.get_entity().entity_type.resource_name
                // );
                let pos = entity
                    .get_entity()
                    .pos
                    .load()
                    .floor_to_i32()
                    .as_packed_chunk_pos();
                chunk_index_tree.insert(pos);
                let subchunk = tracking_section_map.entry(pos).or_default();
                subchunk
                    .entities
                    .push((Arc::downgrade(&entity), entity.get_entity().entity_uuid));
            }
            CacheOp::AddMany { entities } => {
                // continue;
                for entity in entities {
                    let Some(entity): Option<Arc<dyn EntityBase>> = entity.upgrade() else {
                        info!("AM - lost ref");
                        continue;
                    };
                    if Uuid::parse_str("6280580c-8dc8-41c9-b62f-6456c1ec5bc7").unwrap()
                        == entity.get_entity().entity_uuid
                    {
                        info!("added player to cache");
                    }
                    // info!(
                    //     "add entity - {:#?}",
                    //     entity.get_entity().entity_type.resource_name
                    // );
                    let pos = entity
                        .get_entity()
                        .pos
                        .load()
                        .floor_to_i32()
                        .as_packed_chunk_pos();
                    chunk_index_tree.insert(pos);
                    let subchunk = tracking_section_map.entry(pos).or_default();
                    subchunk
                        .entities
                        .push((Arc::downgrade(&entity), entity.get_entity().entity_uuid));
                }
            }
            CacheOp::Remove { pos, entity_uuid } => {
                // info!("remove entity - {}", entity_uuid);
                // continue;
                let pos = pos.floor_to_i32().as_packed_chunk_pos();
                let Some(mut subchunk) = tracking_section_map.get_mut(&pos) else {
                    warn!(
                        "R - tried to remove entity from subchunk that didn't exist in the cache map"
                    );
                    continue;
                };
                let Some(index) = subchunk.entities.iter().position(|e| e.1 == entity_uuid) else {
                    warn!("R - couldn't find entity in subchunk cache");
                    continue;
                };
                subchunk.entities.swap_remove(index);
                dirty_subchunk_indexs.insert(pos);
            }
            CacheOp::Move {
                old_pos,
                new_pos,
                entity_uuid,
            } => {
                // continue;
                // let entity: Option<Arc<dyn EntityBase>> = entity.upgrade();
                // info!("remove entity - {}", entity_uuid);
                let is_player =
                    Uuid::parse_str("6280580c-8dc8-41c9-b62f-6456c1ec5bc7").unwrap() == entity_uuid;
                // if is_player {
                //     info!("moving entity from {:?} to {:?}", old_pos, new_pos);
                //     info!(
                //         "C moving entity from {:?} to {:?}",
                //         old_pos.floor_to_i32().as_packed_chunk_pos(),
                //         new_pos.floor_to_i32().as_packed_chunk_pos()
                //     );
                // }
                let pos = old_pos.floor_to_i32().as_packed_chunk_pos();
                // info!("M - ");
                let Some(mut subchunk) = tracking_section_map.get_mut(&pos) else {
                    if old_pos.floor_to_i32() == Vector3::new(0, 0, 0) {
                        continue;
                    }
                    warn!(
                        "M{} - tried to remove entity from subchunk that didn't exist in the cache map",
                        if is_player { "P" } else { "" }
                    );
                    warn!("moving entity from {:?} to {:?}", old_pos, new_pos);
                    warn!(
                        "C moving entity from {:?} to {:?}",
                        old_pos.floor_to_i32().as_packed_chunk_pos(),
                        new_pos.floor_to_i32().as_packed_chunk_pos()
                    );
                    continue;
                };
                let Some(index) = subchunk.entities.iter().position(|e| e.1 == entity_uuid) else {
                    warn!("M - couldn't find entity in subchunk cache");
                    continue;
                };
                let (entity, _entity_uuid) = subchunk.entities.swap_remove(index);
                dirty_subchunk_indexs.insert(pos);
                // self.dirty_subchunks_tx.send(pos).inspect_err(|er| {
                // error!(
                //     "couldn't send dirty subchunk index in entity cache through channel - {:#?}",
                //     er
                // );
                let Some(entity): Option<Arc<dyn EntityBase>> = entity.upgrade() else {
                    info!("M - lost ref");
                    continue;
                };
                // info!(
                //     "add entity - {:#?}",
                //     entity.get_entity().entity_type.resource_name
                // );
                let pos = new_pos.floor_to_i32().as_packed_chunk_pos();
                chunk_index_tree.insert(pos);
                let subchunk = tracking_section_map.entry(pos).or_default();
                subchunk
                    .entities
                    .push((Arc::downgrade(&entity), entity_uuid));
            }
            CacheOp::CleanCache {
                start,
                is_done_callback,
            } => {
                // is_done_callback.notify_one();
                // continue;
                // info!("cleaning cache");
                // let mut start = Instant::now();
                let mut i = 0u64;
                let mut delay = 0;
                let mx = dirty_subchunk_indexs.len();
                for idx in &dirty_subchunk_indexs {
                    let subchunk_cache = tracking_section_map.get_mut(idx);
                    let is_in_tree = chunk_index_tree.contains(idx);
                    match (subchunk_cache, is_in_tree) {
                        (None, true) => {
                            chunk_index_tree.remove(idx);
                        } //not in the cache but for some reason in the tree - remove from tree
                        (None, false) => {} // not in cache and not in cache - it's fine, working as it should
                        (Some(subchunk_cache), true) => {
                            subchunk_cache.clean();
                            if subchunk_cache.entities.is_empty() {
                                // tracking_section_map.remove(idx);
                                chunk_index_tree.remove(idx);
                            }
                        } // in cache and in tree - look through cache for invalid weaks and filter them out, check for empty cache and remove from tree if empty
                        (Some(subchunk_cache), false) => {
                            subchunk_cache.clean();
                            if subchunk_cache.entities.is_empty() {
                                // tracking_section_map.remove(idx);
                            } else {
                                chunk_index_tree.insert(*idx);
                            }
                        } //in cache but not in tree - clean cache like normal, check for empty cache, add back to tree if it still has something
                    }
                    if start.elapsed().as_millis() - delay >= 50 {
                        warn!("still cleaning cache - {}/{}", i, mx);
                        delay += 10;
                    }
                    i += 1;
                }
                dirty_subchunk_indexs.clear();
                is_done_callback.notify_one();
                // info!("cleaned cache");
            }
            CacheOp::GetEntitiesInBBox {
                bbox,
                return_channel,
            } => {
                let covered_chunks = bbox.covered_chunks();
                let entities = chunk_index_tree
                    .range(
                        covered_chunks.min.as_packed_chunk_pos()
                            ..=covered_chunks.max.as_packed_chunk_pos(),
                    )
                    .par_bridge()
                    .filter_map(|chunk_index| {
                        tracking_section_map.get(chunk_index).map(|subchunk_cache| {
                            subchunk_cache.entities.par_iter().filter_map(|e| {
                                e.0.upgrade().filter(|e: &Arc<dyn EntityBase>| {
                                    e.get_entity().bounding_box.load().intersects(&bbox)
                                })
                            })
                        })
                    })
                    .flatten()
                    .collect::<Vec<_>>();
                return_channel.send(entities).inspect_err(|er| {
                    info!("entity cache failed GetEntitiesInBBox call - couldn't send result back")
                });
            }
        };
        // info!("{} - ran cache op {} left",id,rx.len());
        // info!("{} - finished op {}",id,i);
    }
    info!(
        "entity cache lookup actor has exited due to sender being dropped (likely the world was dropped)"
    );
}

pub struct EntityCache {
    actor_tx: Sender<CacheOp>,
}

/// Cap on how many `CacheOp`s can sit in the actors mailbox before senders start blocking.
/// This exists purely as a memory ceiling for a slow actor (e.g. a weak CPU falling)
const CACHE_OP_QUEUE_CAPACITY: usize = 8192; // <- this variable is to be chosen to be a large enough to not be hit under normal load
// #[derive(Debug)]
enum CacheOp {
    Add {
        entity: Weak<dyn EntityBase>,
    }, //
    AddMany {
        entities: Vec<Weak<dyn EntityBase>>,
    }, //
    Remove {
        pos: Vector3<f64>,
        entity_uuid: Uuid,
    },
    Move {
        old_pos: Vector3<f64>,
        new_pos: Vector3<f64>,
        entity_uuid: Uuid,
    },
    CleanCache {
        start: tokio::time::Instant,
        is_done_callback: Arc<Notify>,
    },
    GetEntitiesInBBox {
        bbox: BoundingBox,
        return_channel: tokio::sync::oneshot::Sender<Vec<Arc<dyn EntityBase>>>,
    },
}

// //packs cords into an i64
// //22 bits for x and z
// //20 bits for y
// //packed as X Z Y
// //logic taken from ChunkSectionPos.java
// fn pack_chunk_cord(x: i32, y: i32, z: i32) -> i64 {
//     ((x & 4194303i32) as i64) << 42
//         | ((y & 1048575i32) as i64) << 0
//         | ((z & 4194303i32) as i64) << 20
// }

// fn unpack_cord_x(packed_cord: i64) -> i32 {
//     (packed_cord << 0 >> 42) as i32
// }

// fn unpack_cord_y(packed_cord: i64) -> i32 {
//     (packed_cord << 44 >> 44) as i32
// }

// fn unpack_cord_z(packed_cord: i64) -> i32 {
//     (packed_cord << 22 >> 42) as i32
// }

//logic taken from SectionedEntityCache.java
//intended use - this is intended to be used in 2 phases, a read phrase and a write phase. all attemped writes and edits are queued till applied by a write phase call
//this 2 phases are to be done such that the read phase is when all chunk, entity and block ticking is done, then after that, the world should start the write phase on this to enact any writes that chunks, entities, blocks or others have attempted, such as an entity moving from one chunk to another
impl EntityCache {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel::<CacheOp>(CACHE_OP_QUEUE_CAPACITY);
        info!(
            "made entitycache with bounded channel ({})",
            CACHE_OP_QUEUE_CAPACITY
        );
        tokio::spawn(actor(rx));
        EntityCache { actor_tx: tx }
    }

    /// Sends a `CacheOp` to the actor, backpressuring the caller instead of dropping the op
    /// when the mailbox is full
    ///
    /// Silently drop Add/Move/Remove do mutate the cache index and loses one
    /// leaves stale or missing entries that a later querie cant self heal
    fn send_cache_op(&self, op: CacheOp) {
        match self.actor_tx.try_send(op) {
            Ok(()) => {}
            Err(TrySendError::Full(op)) => {
                warn!(
                    "entity cache queue full ({} ops) - blocking caller until the actor catches up",
                    CACHE_OP_QUEUE_CAPACITY
                );
                tokio::task::block_in_place(|| {
                    futures::executor::block_on(self.actor_tx.send(op))
                })
                .inspect_err(|_| error!("entity cache actor has died -> dropping"))
                .ok();
            }
            Err(TrySendError::Closed(_)) => {
                error!("entity cache actor has died -> dropping");
            }
        }
    }

    async fn get_entities_in_bbox(&self, bbox: BoundingBox) -> Vec<Arc<dyn EntityBase>> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<Arc<dyn EntityBase>>>();
        self.send_cache_op(CacheOp::GetEntitiesInBBox {
            bbox,
            return_channel: tx,
        });
        rx.await
            .expect("failed to recieve entity cache cmd - GetEntitiesInBBox")
    }

    async fn get_entities_in_bbox_blocking(&self, bbox: BoundingBox) -> Vec<Arc<dyn EntityBase>> {
        tokio::task::block_in_place(|| futures::executor::block_on(self.get_entities_in_bbox(bbox)))
    }

    // async fn for_each_in_box<F>(&self, bbox: BoundingBox, mut entity_cache_chunk_callback: F)
    // where
    //     F: FnMut(
    //         dashmap::mapref::one::Ref<'_, i64, EntityCacheChunk>,
    //     ) -> Pin<Box<dyn Future<Output = ()> + Send>>,
    // {
    //     let chunk_box = bbox.covered_chunks();

    //     let index_tree = self.chunk_index_tree.read().await;
    //     let cache_chunk_iter = (chunk_box.min.x..=chunk_box.max.x)
    //         .flat_map(|chunk_x| {
    //             index_tree
    //                 .range(
    //                     packed_chunk_pos(&Vector3::new(chunk_x, 0, 0))
    //                         ..=packed_chunk_pos(&Vector3::new(chunk_x, -1, -1)),
    //                 )
    //                 .filter(move |packed_chunk_cord| -> bool {
    //                     let chunk_y = unpack_cord_y(**packed_chunk_cord);
    //                     let chunk_z = unpack_cord_z(**packed_chunk_cord);
    //                     chunk_box.contains(chunk_x, chunk_y, chunk_z)
    //                 })
    //         })
    //         .filter_map(|packed_chunk_cord| self.tracking_section_map.get(packed_chunk_cord));

    //     for cache_chunk in cache_chunk_iter {
    //         entity_cache_chunk_callback(cache_chunk).await;
    //     }
    //     //EXPLAINATION
    //     /*
    //     this cache contains 2 structs, a BTreeSet and a DashMap.
    //     the BTreeSet contains packed chunk cords, a way representing a cord to a chunk in an i64.
    //     the BTree is used to quickly check for and get an iter of cords of chunks we are tracking, this helps us to quickly take the huge space of chunks that a box could cover and cut it down to only the chunks we actually have info for.
    //     we then use the resulting packed chunk cords on the DashMap to get structs storing refs to all the entities in that chunk.
    //     all of this is done in an iter with a callback, yay for lazy iters
    //      */
    // }

    pub fn add_entity(&self, entity: Arc<dyn EntityBase>) {
        self.send_cache_op(CacheOp::Add {
            entity: Arc::downgrade(&entity),
        });
    }

    pub fn add_entities(&self, entities: Vec<Arc<dyn EntityBase>>) {
        // info!("add {} entities", entities.len());
        self.send_cache_op(CacheOp::AddMany {
            entities: entities.into_iter().map(|e| Arc::downgrade(&e)).collect(),
        });
    }

    pub fn remove_entity(&self, pos: Vector3<f64>, entity_uuid: Uuid) {
        // info!("remove entity - {}", entity_uuid);
        self.send_cache_op(CacheOp::Remove { pos, entity_uuid });
    }

    // async fn pop_entity(
    //     &self,
    //     pos: Vector3<f64>,
    //     entity_uuid: Uuid,
    // ) -> Option<Arc<dyn EntityBase>> {
    //     info!("pop entity");
    //     let packed_cord = pos.floor_to_i32().as_packed_chunk_pos();
    //     let Some(mut subchunk) = self.tracking_section_map.get_mut(&packed_cord) else {
    //         warn!("tried to remove entity from subchunk that didn't exist in the cache map");
    //         return None;
    //     };
    //     let Some(index) = subchunk.entities.iter().position(|e| e.1 == entity_uuid) else {
    //         warn!("couldn't find entity in subchunk cache");
    //         return None;
    //     };
    //     let popped_entity = subchunk.entities.swap_remove(index);
    //     self.dirty_subchunks_tx.send(packed_cord).inspect_err(|er| {
    //         error!(
    //             "couldn't send dirty subchunk index in entity cache through channel - {:#?}",
    //             er
    //         );
    //     });
    //     popped_entity.0.upgrade()
    // }

    pub fn move_entity(&self, old_pos: Vector3<f64>, new_pos: Vector3<f64>, entity_uuid: Uuid) {
        // info!("move entity");
        self.send_cache_op(CacheOp::Move {
            old_pos,
            new_pos,
            entity_uuid,
        });
    }

    pub fn clean(&self) {
        let notifer = Arc::new(tokio::sync::Notify::new());
        self.send_cache_op(CacheOp::CleanCache {
            start: tokio::time::Instant::now(),
            is_done_callback: notifer.clone(),
        });
        // info!("cache clean callback - is termed : {}",rx.is_terminated());
        // info!("cache clean callback - is empty : {}",rx.is_empty());
        // let t = rx.await;
        // info!("cache clean callback received");
        // info!("cache clean callback response : {:#?}",t);
        // info!("waiting for clean to finish");
        tokio::task::block_in_place(|| futures::executor::block_on(notifer.notified()))
        // match t {
        //     Ok(_) => info!("done good"),
        //     Err(er) => info!("callback go bad : {}",er),
        // }
    }

    // fn add(&self, packed_chunk_cord: i64, entity: Weak<dyn EntityBase>) {
    //     let mut entity_cache_chunk = if let Some(entity_cache_chunk_tmp) =
    //         self.tracking_section_map.get_mut(&packed_chunk_cord)
    //     {
    //         entity_cache_chunk_tmp
    //     } else {
    //         self.tracking_section_map
    //             .insert(packed_chunk_cord, EntityCacheChunk::default());
    //         self.tracking_section_map
    //             .get_mut(&packed_chunk_cord)
    //             .expect("we just inserted a blank one")
    //     };
    //     entity_cache_chunk.entities.push(entity);
    // }

    // pub async fn apply_queued_ops(&self) {
    //     let dirty_chunks: DashSet<i64> = DashSet::new();
    //     let mut queued_ops_rx = self.queued_ops_rx.lock().await;

    //     if !queued_ops_rx.is_empty() {
    //         let mut adds = 0;
    //         let mut addsmults = 0;
    //         let mut addsmultsamttotal = 0;
    //         let mut removes = 0;
    //         let mut moves = 0;

    //         let mut buf: Vec<CacheOp> = Vec::new();
    //         info!("receiver says there are {} ops",queued_ops_rx.len());
    //         let tmp = queued_ops_rx.recv_many(&mut buf, 10240).await;
    //         info!("cache op gather returned {}",tmp);
    //         info!("going through {} cache ops",buf.len());
    //         drop(queued_ops_rx);
    //         for op in buf {
    //             match op {
    //                 CacheOp::Add { entity } => {
    //                     adds += 1;
    //                     if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
    //                         Some(
    //                             ent.get_entity()
    //                                 .pos
    //                                 .load()
    //                                 .floor_to_i32()
    //                                 .as_packed_chunk_pos(),
    //                         )
    //                     }) {
    //                         self.add(packed_chunk_cord, entity);
    //                     } else {
    //                         warn!("tried to add entity that could not be upgraded");
    //                     }
    //                 }
    //                 CacheOp::AddMany { entities } => {
    //                     addsmults += 1;
    //                     addsmultsamttotal += entities.len();
    //                     entities
    //                         .into_par_iter()
    //                         .for_each(|entity: Weak<dyn EntityBase>| {
    //                             if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
    //                                 Some(
    //                                     ent.get_entity()
    //                                         .pos
    //                                         .load()
    //                                         .floor_to_i32()
    //                                         .as_packed_chunk_pos(),
    //                                 )
    //                             }) {
    //                                 self.add(packed_chunk_cord, entity);
    //                             } else {
    //                                 warn!("tried to add entity that could not be upgraded");
    //                             }
    //                         })
    //                 }
    //                 CacheOp::Remove { entity_pos } => {
    //                     removes += 1;
    //                     let packed_chunk_cord = entity_pos.floor_to_i32().as_packed_chunk_pos();
    //                     dirty_chunks.insert(packed_chunk_cord);
    //                 }
    //                 CacheOp::Move { old_pos, entity } => {
    //                     moves += 1;
    //                     //attempt removal of the entity from it's old position
    //                     let packed_chunk_cord = old_pos.floor_to_i32().as_packed_chunk_pos();
    //                     dirty_chunks.insert(packed_chunk_cord);

    //                     //attempt adding to it's new pos
    //                     if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
    //                         Some(
    //                             ent.get_entity()
    //                                 .pos
    //                                 .load()
    //                                 .floor_to_i32()
    //                                 .as_packed_chunk_pos(),
    //                         )
    //                     }) {
    //                         self.add(packed_chunk_cord, entity);
    //                     }
    //                 }
    //             }
    //         }
    //         info!("Added {} entities",adds);
    //         info!("Added {} entities via {} vecs",addsmultsamttotal,addsmults);
    //         info!("removed {} entities",removes);
    //         info!("moved {} entities",moves);
    //         info!("marked {} subchunks as dirt",dirty_chunks.len());
    //         let mut removedEntitys = 0;
    //         dirty_chunks.into_iter().for_each(|packed_chunk_cord| {
    //             self.tracking_section_map
    //                 .get_mut(&packed_chunk_cord)
    //                 .and_then(|mut entity_tracking_chunk| -> Option<()> {
    //                     let ol = entity_tracking_chunk.entities.len();
    //                     entity_tracking_chunk.entities = entity_tracking_chunk
    //                         .entities
    //                         .par_iter()
    //                         .filter(|entity| entity.upgrade().is_some())
    //                         .cloned()
    //                         .collect();
    //                     removedEntitys+=entity_tracking_chunk.entities.len()-ol;
    //                     None
    //                 });
    //         });
    //         info!("removed {} entities from dirty subchunks",removedEntitys);
    //         let ol = self.tracking_section_map.len();
    //         self.tracking_section_map
    //             .retain(|_, va| !va.entities.is_empty());
    //         info!("removed {} empty subchunks",ol-self.tracking_section_map.len());
    //     }
    // }
}

#[derive(Default)]
pub struct EntityCacheChunk {
    pub entities: Vec<(Weak<dyn EntityBase>, Uuid)>,
}

impl EntityCacheChunk {
    pub fn clean(&mut self) {
        self.entities = self
            .entities
            .iter()
            .filter_map(|e| e.0.upgrade().map(|a| (a, e.1)))
            .map(|e| (Arc::downgrade(&e.0), e.1))
            .collect();
    }
}
