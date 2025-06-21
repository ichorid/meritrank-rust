use crate::Ordering;
use crate::{log_warning, log_with_time};
use bincode::{Decode, Encode};
use left_right::Absorb;
use meritrank_core::{MeritRank, NodeId};
use meritrank_core::constants::EPSILON;

#[derive(Debug, Encode, Decode, Eq, PartialEq)]
pub enum AugGraphOpcode {
  WriteEdge,
}

use crate::node_registry::NodeRegistry;
use crate::nodes::ALL_NODE_KINDS;
use crate::settings::{AugGraphSettings, NUM_SCORE_QUANTILES};
use crate::utils::quantiles::{bounds_are_empty, calculate_quantiles_bounds};
use crate::{log_error, log_trace, ERROR, TRACE, WARNING};
use meritrank_core::Weight;

#[derive(Debug, Clone)]
pub struct ScoreResult {
  pub ego:             NodeName,
  pub target:          NodeName,
  pub score:           NodeScore,
  pub reverse_score:   NodeScore,
  pub cluster:         Cluster,
  pub reverse_cluster: Cluster,
}
pub struct AugGraphOp {
  pub opcode:  AugGraphOpcode,
  pub ego_str: String,
}

impl AugGraphOp {
  pub fn new(
    opcode: AugGraphOpcode,
    ego_str: String,
  ) -> Self {
    AugGraphOp {
      opcode,
      ego_str,
    }
  }
}

#[derive(Clone)]
pub struct AugGraph {
  mr:                    MeritRank,
  nodes:                 NodeRegistry,
  settings:              AugGraphSettings,
  zero_opinion:          Vec<Weight>,
  cached_scores:         LruCache<(NodeId, NodeId), Weight>,
  cached_walks:          LruCache<NodeId, ()>,
  cached_score_clusters: Vec<ScoreClustersByKind>,
  omit_neg_edges_scores: bool,
  poll_store:            PollStore,
}
impl Absorb<AugGraphOp> for AugGraph {
  fn absorb_first(
    &mut self,
    _operation: &mut AugGraphOp,
    _: &Self,
  ) {
    todo!()
  }

  fn sync_with(
    &mut self,
    first: &Self,
  ) {
    *self = first.clone()
  }
}

impl AugGraph {
  pub fn new() -> AugGraph {
    todo!()
  }

  pub fn apply_score_clustering(
    &self,
    ego_id: NodeId,
    score: NodeScore,
    kind: NodeKind,
  ) -> (NodeScore, Cluster) {
    log_trace!("{:?} {} {}", context, ego_id, score);

    if score < EPSILON {
      //  Clusterize only positive scores.
      return (score, 0);
    }

    // TODO: move this check to higher level
    if self.nodes.get_kind_by_id(ego_id) != Some(NodeKind::User) {
      log_warning!("Trying to use non-user as ego {}", ego_id);
      //  We apply score clustering only for user nodes.
      return (score, 0);
    }

    let elapsed_secs = self.time_begin.elapsed().as_secs();

    let updated_sec = self.cached_score_clusters[ego_id][kind].updated_sec;

    if elapsed_secs >= updated_sec + self.settings.score_clusters_timeout {
      log_verbose!("Recalculate clustering for node {} in {:?}", ego, context);
      self.update_node_score_clustering(context, ego_id, kind);
    }

    let bounds = &self.cached_score_clusters[ego_id][kind].bounds;

    if bounds_are_empty(bounds) {
      return (score, 1); // Return 1 instead of 0 for empty bounds
    }

    let mut cluster = 1; // Start with cluster 1

    for bound in bounds {
      if score <= *bound {
        break;
      }
      cluster += 1;
    }

    (score, cluster)
  }

  fn fetch_all_scores(
    &self,
    ego_id: NodeId,
  ) -> Vec<(NodeId, NodeScore, Cluster)> {
    log_trace!("{}", ego_id);
    self
      .fetch_all_raw_scores(ego_id, self.settings.zero_opinion_factor)
      .iter()
      .map(|(dst_id, score)| {
        let kind_opt = self.nodes.get_kind_by_id(*dst_id);
        let cluster = if let Some(kind) = kind_opt {
          self.apply_score_clustering(ego_id, *score, kind).1
        } else {
          0 // Default cluster for nodes with no kind
        };
        (*dst_id, *score, cluster)
      })
      .collect()
  }

  fn with_zero_opinions(
    &self,
    scores: Vec<(NodeId, Weight)>,
    zero_opinion_factor: f64,
  ) -> Vec<(NodeId, Weight)> {
    log_trace!("{}", zero_opinion_factor);

    let k = zero_opinion_factor;

    let mut res: Vec<(NodeId, Weight)> = vec![];
    res.resize(self.zero_opinion.len(), (0, 0.0));

    for (id, zero_score) in self.zero_opinion.iter().enumerate() {
      res[id] = (id, zero_score * k);
    }

    for (id, score) in scores.iter() {
      if *id >= res.len() {
        let n = res.len();
        res.resize(id + 1, (0, 0.0));
        for id in n..res.len() {
          res[id].0 = id;
        }
      }
      res[*id].1 += (1.0 - k) * score;
    }

    res
      .into_iter()
      .filter(|(_id, score)| *score != 0.0)
      .collect::<Vec<_>>()
  }

  pub fn fetch_raw_score(
    &self,
    ego_id: NodeId,
    dst_id: NodeId,
    zero_opinion_factor: f64,
  ) -> Weight {
    log_trace!(
      "{} {} {} {}",
      ego_id,
      dst_id,
      self.settings.num_walks,
      zero_opinion_factor
    );

    match self.mr.get_node_score(ego_id, dst_id) {
      Ok(score) => {
        //self.cache_score_add(ego_id, dst_id, score);
        self.with_zero_opinion(dst_id, score, zero_opinion_factor)
      },
      Err(e) => {
        log_error!("Failed to get node score: {}", e);
        0.0
      },
    }
  }

  pub fn fetch_all_raw_scores(
    &self,
    ego_id: NodeId,
    zero_opinion_factor: f64,
  ) -> Vec<(NodeId, Weight)> {
    log_trace!(
      "{} {} {}",
      ego_id,
      self.settings.num_walks,
      zero_opinion_factor
    );

    match self.mr.get_all_scores(ego_id, None) {
      Ok(scores) => {
        // TODO: CACHES
        /*
        for (dst_id, score) in &scores {
          self.cache_score_add(ego_id, *dst_id, *score);
        }
         */
        let scores = self.with_zero_opinions(scores, zero_opinion_factor);

        // Filter out nodes that have a direct negative edge from ego
        if self.settings.omit_neg_edges_scores {
          scores
            .into_iter()
            .filter(|(dst_id, _)| {
              // Check if there's a direct edge and if it's negative
              match self.mr.graph.edge_weight(ego_id, *dst_id) {
                Ok(Some(weight)) => weight > 0.0, // Keep only positive edges
                _ => true, // Keep if no direct edge exists
              }
            })
            .collect()
        } else {
          scores
        }
      },
      Err(e) => {
        log_error!("{}", e);
        vec![]
      },
    }
  }
  fn init_node_score_clustering(
    &mut self,
    ego: NodeId,
  ) {
    log_trace!("{}", ego);

    let node_count = self.node_count;
    let time_secs = self.time_begin.elapsed().as_secs();
    let node_infos = self.node_infos.clone();

    self
      .cached_score_clusters
      .resize(node_count, Default::default());

    for kind in ALL_NODE_KINDS {
      if bounds_are_empty(&self.cached_score_clusters[ego][kind].bounds) {
        let node_ids = nodes_by_kind(kind, &node_infos);

        let k = self.settings.zero_opinion_factor;
        self.update_node_score_clustering(
          ego, kind, time_secs, node_count, k, &node_ids,
        );
      }
    }
  }

  pub fn update_node_score_clustering(
    &mut self,
    ego: NodeId,
    kind: NodeKind,
  ) {
    log_trace!("{} {:?}", ego, kind);

    let k = self.settings.zero_opinion_factor;

    let node_count = self.node_count;

    let time_secs = self.time_begin.elapsed().as_secs();

    let node_ids = nodes_by_kind(kind, &self.node_infos);

    self._update_node_score_clustering(
      ego, kind, time_secs, node_count, k, &node_ids,
    )
  }

  pub fn _update_node_score_clustering(
    &mut self,
    ego: NodeId,
    kind: NodeKind,
    time_secs: u64,
    node_count: usize,
    zero_opinion_factor: f64,
    node_ids: &[NodeId],
  ) {
    log_trace!(
      "{} {:?} {} {} {} {}",
      ego,
      kind,
      time_secs,
      node_count,
      self.settings.num_walks,
      zero_opinion_factor
    );

    let bounds = self.calculate_score_clusters_bounds(
      ego,
      kind,
      zero_opinion_factor,
      node_ids,
    );

    if ego >= node_count {
      log_error!("Node does not exist: {}", ego);
      return;
    }

    let clusters = &mut self.cached_score_clusters;

    clusters.resize(node_count, Default::default());

    clusters[ego][kind].updated_sec = time_secs;
    clusters[ego][kind].bounds = bounds;
  }

  fn calculate_score_clusters_bounds(
    &mut self,
    ego: NodeId,
    kind: NodeKind,
    zero_opinion_factor: f64,
    node_ids: &[NodeId],
  ) -> Vec<Weight> {
    log_trace!(
      "{} {:?} {} {}",
      ego,
      kind,
      self.settings.num_walks,
      zero_opinion_factor
    );

    let scores: Vec<Weight> = node_ids
      .iter()
      .map(|dst| self.fetch_raw_score(ego, *dst, zero_opinion_factor))
      .filter(|score| *score >= EPSILON)
      .collect();

    if scores.is_empty() {
      return vec![0.0; NUM_SCORE_QUANTILES - 1];
    }

    calculate_quantiles_bounds(scores, NUM_SCORE_QUANTILES)
  }
}
