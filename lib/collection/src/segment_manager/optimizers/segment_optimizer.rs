use segment::types::{PointIdType};
use crate::collection::OperationResult;
use crate::segment_manager::holders::segment_holder::{SegmentId, LockedSegment, LockerSegmentHolder};
use std::sync::{Arc, RwLock};
use segment::segment::Segment;
use std::collections::HashSet;
use crate::segment_manager::holders::proxy_segment::ProxySegment;
use segment::entry::entry_point::SegmentEntry;

pub trait SegmentOptimizer {
    /// Checks if segment optimization is required
    fn check_condition(&self, segments: LockerSegmentHolder) -> Vec<SegmentId>;

    /// Build temp segment
    fn temp_segment(&self) -> LockedSegment;

    /// Build optimized segment
    fn optimized_segment(&self) -> Segment;


    /// Performs optimization of collections's segments, including:
    ///     - Segment rebuilding
    ///     - Segment joining
    fn optimize(&self, segments: LockerSegmentHolder, ids: Vec<SegmentId>) -> OperationResult<bool> {
        let tmp_segment = self.temp_segment();

        let proxy_deleted_points = Arc::new(RwLock::new(HashSet::<PointIdType>::new()));

        let optimizing_segments: Vec<_> = {
            let read_segments = segments.read().unwrap();
            ids.iter().cloned()
                .map(|id| read_segments.get(id))
                .filter_map(|x| x.and_then(|x| Some(x.mk_copy()) ))
                .collect()
        };

        let proxies: Vec<_> = optimizing_segments.iter()
            .map(|sg| ProxySegment::new(
                sg.mk_copy(),
                tmp_segment.mk_copy(),
                proxy_deleted_points.clone(),
            )).collect();


        let proxy_ids: Vec<_> = {
            let mut write_segments = segments.write().unwrap();
            proxies.into_iter()
                .zip(ids.iter().cloned())
                .map(|(proxy, idx)| write_segments.swap(proxy, &vec![idx]))
                .collect()
        };

        let mut optimized_segment = self.optimized_segment();


        // ---- SLOW PART -----
        for segment in optimizing_segments {
            let segment_guard = segment.0.read().unwrap();
            optimized_segment.update_from(&*segment_guard);
        }

        // Delete points in 2 steps
        // First step - delete all points with read lock
        // Second step - delete all the rest points with full write lock
        let deleted_points_snapshot: HashSet<PointIdType> = proxy_deleted_points.read().unwrap().iter().cloned().collect();
        for point_id in deleted_points_snapshot.iter().cloned() {
            optimized_segment.delete_point(
                optimized_segment.version,
                point_id,
            ).unwrap();
        }
        optimized_segment.build_index();
        // ---- SLOW PART ENDS HERE -----

        { // This block locks all operations with collection. It should be fast
            let mut write_segments = segments.write().unwrap();
            let deleted_points = proxy_deleted_points.read().unwrap();
            let points_diff = deleted_points_snapshot.difference(&deleted_points);
            for point_id in points_diff.into_iter() {
                optimized_segment.delete_point(
                    optimized_segment.version,
                    *point_id,
                ).unwrap();
            }
            write_segments.swap(optimized_segment, &proxy_ids);
            if tmp_segment.0.read().unwrap().vectors_count() > 0 { // Do not add temporary segment if no points changed
                write_segments.add_locked(tmp_segment);
            }
        }


        Ok(true)
    }
}