use std::{
    collections::{BTreeSet, HashSet},
    pin::Pin,
    sync::{Arc, Weak},
};

use dashmap::{DashMap, DashSet};
use pumpkin_util::math::{
    block_box::BlockBox,
    bounding_box::BoundingBox,
    vector3::{Vector3, packed_chunk_pos},
};
use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator,
};
use tokio::{
    pin,
    sync::{
        Mutex, RwLock,
        mpsc::{UnboundedReceiver, UnboundedSender},
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



pub struct EntityCache {
    tracking_section_map: DashMap<i64, EntityCacheChunk>,
    chunk_index_tree: RwLock<BTreeSet<i64>>,
    queued_ops_tx: UnboundedSender<CacheOp>,
    queued_ops_rx: Mutex<UnboundedReceiver<CacheOp>>,
}

enum CacheOp {
    Add {
        entity: Weak<dyn EntityBase>,
    }, //
    AddMany {
        entities: Vec<Weak<dyn EntityBase>>,
    }, //
    Remove {
        entity_pos: Vector3<f64>,
    },
    Move {
        old_pos: Vector3<f64>,
        entity: Weak<dyn EntityBase>,
    },
}


//packs cords into an i64
//22 bits for x and z
//20 bits for y
//packed as X Z Y
//logic taken from ChunkSectionPos.java
fn pack_chunk_cord(x: i32, y: i32, z: i32) -> i64 {
    ((x & 4194303i32) as i64) << 42
        | ((y & 1048575i32) as i64) << 0
        | ((z & 4194303i32) as i64) << 20
}

fn unpack_cord_x(packed_cord: i64) -> i32 {
    (packed_cord << 0 >> 42) as i32
}

fn unpack_cord_y(packed_cord: i64) -> i32 {
    (packed_cord << 44 >> 44) as i32
}

fn unpack_cord_z(packed_cord: i64) -> i32 {
    (packed_cord << 22 >> 42) as i32
}

//logic taken from SectionedEntityCache.java
//intended use - this is intended to be used in 2 phases, a read phrase and a write phase. all attemped writes and edits are queued till applied by a write phase call
//this 2 phases are to be done such that the read phase is when all chunk, entity and block ticking is done, then after that, the world should start the write phase on this to enact any writes that chunks, entities, blocks or others have attempted, such as an entity moving from one chunk to another
impl EntityCache {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<CacheOp>();
        info!("made entitycache");
        EntityCache {
            tracking_section_map: DashMap::new(),
            chunk_index_tree: RwLock::new(BTreeSet::new()),
            queued_ops_tx: tx,
            queued_ops_rx: Mutex::new(rx),
        }
    }

    async fn for_each_in_box<F>(&self, bbox: BoundingBox, mut entity_cache_chunk_callback: F)
    where
        F: FnMut(
            dashmap::mapref::one::Ref<'_, i64, EntityCacheChunk>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>,
    {
        let chunk_box = bbox.covered_chunks();

        let index_tree = self.chunk_index_tree.read().await;
        let cache_chunk_iter = (chunk_box.min.x..=chunk_box.max.x)
            .flat_map(|chunk_x| {
                index_tree
                    .range(
                        packed_chunk_pos(&Vector3::new(chunk_x, 0, 0))
                            ..=packed_chunk_pos(&Vector3::new(chunk_x, -1, -1)),
                    )
                    .filter(move |packed_chunk_cord| -> bool {
                        let chunk_y = unpack_cord_y(**packed_chunk_cord);
                        let chunk_z = unpack_cord_z(**packed_chunk_cord);
                        chunk_box.contains(chunk_x, chunk_y, chunk_z)
                    })
            })
            .filter_map(|packed_chunk_cord| self.tracking_section_map.get(packed_chunk_cord));

        for cache_chunk in cache_chunk_iter {
            entity_cache_chunk_callback(cache_chunk).await;
        }
        //EXPLAINATION
        /*
        this cache contains 2 structs, a BTreeSet and a DashMap.
        the BTreeSet contains packed chunk cords, a way representing a cord to a chunk in an i64.
        the BTree is used to quickly check for and get an iter of cords of chunks we are tracking, this helps us to quickly take the huge space of chunks that a box could cover and cut it down to only the chunks we actually have info for.
        we then use the resulting packed chunk cords on the DashMap to get structs storing refs to all the entities in that chunk.
        all of this is done in an iter with a callback, yay for lazy iters
         */
    }

    pub fn add_entity(&self, entity: Weak<dyn EntityBase>) {
        let tn: Arc<dyn EntityBase> = entity.upgrade().unwrap();
        info!(
            "add entity - {:#?}",
            tn.get_entity().entity_type.resource_name
        );
        (self.queued_ops_tx.send(CacheOp::Add { entity }));
    }

    pub fn add_entities(&self, entities: Vec<Weak<dyn EntityBase>>) {
        info!("add {} entities", entities.len());
        (self.queued_ops_tx.send(CacheOp::AddMany { entities }));
    }

    pub fn remove_entity(&self, entity: &dyn EntityBase) {
        info!(
            "remove entity - {}",
            entity.get_entity().entity_type.resource_name
        );
        (self.queued_ops_tx.send(CacheOp::Remove {
            entity_pos: entity.get_entity().pos.load(),
        }));
    }

    pub fn move_entity(&self, old_pos: Vector3<f64>, entity: Weak<dyn EntityBase>) {
        info!("move entity");
        (self.queued_ops_tx.send(CacheOp::Move { old_pos, entity }));
    }

    fn add(&self, packed_chunk_cord: i64, entity: Weak<dyn EntityBase>) {
        let mut entity_cache_chunk = if let Some(entity_cache_chunk_tmp) =
            self.tracking_section_map.get_mut(&packed_chunk_cord)
        {
            entity_cache_chunk_tmp
        } else {
            self.tracking_section_map
                .insert(packed_chunk_cord, EntityCacheChunk::default());
            self.tracking_section_map
                .get_mut(&packed_chunk_cord)
                .expect("we just inserted a blank one")
        };
        entity_cache_chunk.entities.push(entity);
    }

    pub async fn apply_queued_ops(&self) {
        let dirty_chunks: DashSet<i64> = DashSet::new();
        let mut queued_ops_rx = self.queued_ops_rx.lock().await;

        if !queued_ops_rx.is_empty() {
            let mut adds = 0;
            let mut addsmults = 0;
            let mut addsmultsamttotal = 0;
            let mut removes = 0;
            let mut moves = 0;
            
            let mut buf: Vec<CacheOp> = Vec::new();
            info!("receiver says there are {} ops",queued_ops_rx.len());
            let tmp = queued_ops_rx.recv_many(&mut buf, 10240).await;
            info!("cache op gather returned {}",tmp);
            info!("going through {} cache ops",buf.len());
            drop(queued_ops_rx);
            for op in buf {
                match op {
                    CacheOp::Add { entity } => {
                        adds += 1;
                        if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
                            Some(
                                ent.get_entity()
                                    .pos
                                    .load()
                                    .floor_to_i32()
                                    .as_packed_chunk_pos(),
                            )
                        }) {
                            self.add(packed_chunk_cord, entity);
                        } else {
                            warn!("tried to add entity that could not be upgraded");
                        }
                    }
                    CacheOp::AddMany { entities } => {
                        addsmults += 1;
                        addsmultsamttotal += entities.len();
                        entities
                            .into_par_iter()
                            .for_each(|entity: Weak<dyn EntityBase>| {
                                if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
                                    Some(
                                        ent.get_entity()
                                            .pos
                                            .load()
                                            .floor_to_i32()
                                            .as_packed_chunk_pos(),
                                    )
                                }) {
                                    self.add(packed_chunk_cord, entity);
                                } else {
                                    warn!("tried to add entity that could not be upgraded");
                                }
                            })
                    }
                    CacheOp::Remove { entity_pos } => {
                        removes += 1;
                        let packed_chunk_cord = entity_pos.floor_to_i32().as_packed_chunk_pos();
                        dirty_chunks.insert(packed_chunk_cord);
                    }
                    CacheOp::Move { old_pos, entity } => {
                        moves += 1;
                        //attempt removal of the entity from it's old position
                        let packed_chunk_cord = old_pos.floor_to_i32().as_packed_chunk_pos();
                        dirty_chunks.insert(packed_chunk_cord);

                        //attempt adding to it's new pos
                        if let Some(packed_chunk_cord) = entity.upgrade().and_then(|ent| {
                            Some(
                                ent.get_entity()
                                    .pos
                                    .load()
                                    .floor_to_i32()
                                    .as_packed_chunk_pos(),
                            )
                        }) {
                            self.add(packed_chunk_cord, entity);
                        }
                    }
                }
            }
            info!("Added {} entities",adds);
            info!("Added {} entities via {} vecs",addsmultsamttotal,addsmults);
            info!("removed {} entities",removes);
            info!("moved {} entities",moves);
            info!("marked {} subchunks as dirt",dirty_chunks.len());
            let mut removedEntitys = 0;
            dirty_chunks.into_iter().for_each(|packed_chunk_cord| {
                self.tracking_section_map
                    .get_mut(&packed_chunk_cord)
                    .and_then(|mut entity_tracking_chunk| -> Option<()> {
                        let ol = entity_tracking_chunk.entities.len();
                        entity_tracking_chunk.entities = entity_tracking_chunk
                            .entities
                            .par_iter()
                            .filter(|entity| entity.upgrade().is_some())
                            .cloned()
                            .collect();
                        removedEntitys+=entity_tracking_chunk.entities.len()-ol;
                        None
                    });
            });
            info!("removed {} entities from dirty subchunks",removedEntitys);
            let ol = self.tracking_section_map.len();
            self.tracking_section_map
                .retain(|_, va| !va.entities.is_empty());
            info!("removed {} empty subchunks",ol-self.tracking_section_map.len());
        }
    }
}

#[derive(Default)]
pub struct EntityCacheChunk {
    pub entities: Vec<Weak<dyn EntityBase>>,
}
