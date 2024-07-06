use rand::distributions::WeightedIndex;
use rand::prelude::*;

use integer_hasher::IntMap;
use tinyset::SetUsize;
use crate::constants::EPSILON;
use crate::common::sign;
use crate::constants::{ASSERT, OPTIMIZE_INVALIDATION};
use crate::counter::Counter;
use crate::errors::MeritRankError;
use crate::graph::{Graph, NodeId, Weight};
use crate::random_walk::RandomWalk;
use crate::walk_storage::{WalkId, WalkStorage};

pub struct MeritRank<NodeData : Copy + Default> {
  pub graph         : Graph<NodeData>,
      walks         : WalkStorage,
      personal_hits : IntMap<NodeId, Counter>,
      neg_hits      : IntMap<NodeId, IntMap<NodeId, Weight>>,
  pub alpha         : Weight,
}

#[derive(PartialEq, Eq)]
pub enum Neighbors {
  All,
  Positive,
  Negative,
}


/// Updates the negative hits based on a random walk and negative penalties.
///
/// This method updates the negative hit counts for each node in the `walk` based on the penalties
/// specified in the `negs` hashmap. The `subtract` flag determines whether the penalties should be added
/// or subtracted from the hit counts.
///
/// # Arguments
///
/// * `walk` - The random walk for which to update the negative hits.
/// * `negs` - A hashmap containing the negative penalties for each node.
/// * `subtract` - A boolean flag indicating whether to subtract the penalties from the hit counts.
pub fn update_negative_hits(
    neg_hits: &mut IntMap<NodeId, IntMap<NodeId, Weight>>,
    walk: &RandomWalk,
    negs: &IntMap<NodeId, Weight>,
    subtract: bool,
) {
    // TODO: optimize the intersection - walk members should be checked against negs
    if walk.intersects_nodes(negs.keys()) {
        let ego_neg_hits = neg_hits
            .entry(walk.first_node().unwrap())
            .or_insert_with(IntMap::default);

        for (node, penalty) in walk.calculate_penalties(negs) {
            let adjusted_penalty = if subtract { -penalty } else { penalty };
            let entry = ego_neg_hits.entry(node).or_insert(0.0);
            *entry += adjusted_penalty;
        }
    }
}


/// Clears an invalidated walk by subtracting the invalidated segment nodes from the hit counter.
///
/// This method clears an invalidated walk by subtracting the nodes in the invalidated segment from the hit counter
/// for the starting node of the invalidated walk. It ensures that the hit counter values remain non-negative.
/// The invalidated segment may include nodes that are still present in the original walk, so special care is taken
/// to avoid subtracting them from the counter by accident.
///
/// # Arguments
///
/// * `walk` - The invalidated walk.
/// * `invalidated_segment` - The list of invalidated segment nodes.
pub fn revert_counters_for_walk_from_pos(
    personal_hits: &mut IntMap<NodeId, Counter>,
    walk: &RandomWalk,
    pos: usize,
) {
    // Get the starting node (ego) of the invalidated walk
    let ego = walk.first_node().unwrap(); // Assuming first_node() returns NodeId

    // Get or insert the hit counter for the starting node
    let counter = personal_hits.entry(ego).or_insert_with(Counter::new);

    // Collect nodes before pos into a set for efficient membership checking
    let nodes_before_pos = &walk.get_nodes()[..pos];

    // Remove nodes after pos that were not visited before pos
    let mut nodes_to_remove = SetUsize::new();
    for &node in &walk.get_nodes()[pos..] {
        if !nodes_before_pos.contains(&node) {
            nodes_to_remove.insert(node);
        }
    }

    // Adjust counters for nodes to remove
    if !nodes_to_remove.is_empty() {
        for node_to_remove in nodes_to_remove {
            *counter.get_mut_count(&node_to_remove) -= 1.0;
        }

        // Debug assertion to check if hit counter values are non-negative
        #[cfg(debug_assertions)]
        for &c in counter.count_values() {
            assert!(c >= 0.0);
        }
    }
}


// #[allow(dead_code)]
impl<NodeData : Copy + Default> MeritRank<NodeData> {
  /// Creates a new `MeritRank` instance with the given graph.
  ///
  /// # Arguments
  ///
  /// * `graph` - A `Graph` instance representing the underlying graph.
  ///
  /// # Returns
  ///
  /// * `Result<Self, MeritRankError>` - A `Result` indicating success (`Ok`) or an error (`Err`) if the graph contains a self-reference.
  pub fn new(graph : Graph<NodeData>) -> Result<Self, MeritRankError> {
    // Check if the graph contains a self-reference
    if let Err(err) = graph.check_self_reference() {
      return Err(err);
    }

    Ok(MeritRank {
      graph,
      walks: WalkStorage::new(),
      personal_hits: IntMap::default(),
      neg_hits: IntMap::default(),
      alpha: 0.85,
    })
  }

  fn _get_neg_hits(&self) -> &IntMap<NodeId, IntMap<NodeId, Weight>> {
    &self.neg_hits
  }

  fn _get_personal_hits(&self) -> &IntMap<NodeId, Counter> {
    &self.personal_hits
  }

  // Get the hit count for a specific node
  fn _get_hit_counts(&self, node: &NodeId) -> Option<f64> {
    self.personal_hits
      .get(node)
      .and_then(|counter| counter.get_count(node))
      .map(|&count| count as f64)
  }

  /// Retrieves the weighted neighbors of a node.
  ///
  /// This method returns a hashmap of the neighbors of the specified `node`, along with their weights.
  /// Only neighbors with positive weights are returned if `positive` is `true`, and only neighbors with negative
  /// weights are returned if `positive` is `false`.
  ///
  /// # Arguments
  ///
  /// * `node` - The node for which to retrieve the neighbors.
  /// * `positive` - A boolean value indicating whether to return positive neighbors.
  ///
  /// # Returns
  ///
  /// A hashmap of the neighbors of the specified `node` and their weights, or `None` if no neighbors exist.
  ///
  /// # Examples
  ///
  /// ```
  /// use meritrank::{Graph, NodeId, MeritRankError, MeritRank, Neighbors};
  ///
  /// let graph = Graph::<()>::new();
  /// let merit_rank = MeritRank::new(graph).unwrap();
  ///
  /// let node : NodeId = 1;
  ///
  /// if let Some(neighbors) = merit_rank.neighbors_weighted(node, Neighbors::Positive) {
  ///   for (neighbor, weight) in neighbors {
  ///     println!("Neighbor: {:?}, Weight: {:?}", neighbor, weight);
  ///   }
  /// } else {
  ///   println!("No neighbors found for the node.");
  /// }
  /// ```
  pub fn neighbors_weighted(
    &self,
    node  : NodeId,
    mode  : Neighbors,
  ) -> Option<IntMap<NodeId, Weight>> {
    let neighbors: IntMap<_, _> = self
      .graph
      .neighbors(node)
      .into_iter()
      .filter_map(|nbr| {
        let weight = self.graph.edge_weight(node, nbr).unwrap_or_else(|| 0.0);
        if  mode == Neighbors::All                       ||
           (mode == Neighbors::Positive && weight > 0.0) ||
           (mode == Neighbors::Negative && weight < 0.0) {
          Some((nbr, weight))
        } else {
          None
        }
      })
      .collect();

    if neighbors.is_empty() {
      None
    } else {
      Some(neighbors)
    }
  }

  /// Calculates the MeritRank from the perspective of the given node.
  ///
  /// If there are already walks for the node, they are dropped, and a new calculation is performed.
  ///
  /// # Arguments
  ///
  /// * `ego` - The source node to calculate the MeritRank for.
  /// * `num_walks` - The number of walks that should be used.
  ///
  /// # Returns
  ///
  /// * `Result<(), MeritRankError>` - A `Result` indicating success (`Ok`) or an error (`Err`) if the node does not exist.
  ///
  /// # Errors
  ///
  /// An error is returned if the specified node does not exist in the graph.
  ///
  /// # Example
  ///
  /// ```rust
  /// use meritrank::{MeritRank, MeritRankError, Graph, NodeId};
  ///
  /// let graph = Graph::<()>::new();
  /// let mut merit_rank = MeritRank::new(graph).unwrap();
  ///
  /// let ego : NodeId = 1;
  /// let num_walks = 1000;
  ///
  /// if let Err(err) = merit_rank.calculate(ego, num_walks) {
  ///   match err {
  ///     MeritRankError::NodeDoesNotExist => {
  ///       println!("Error: The specified node does not exist.");
  ///     }
  ///     // Handle other error cases...
  ///   _ => {}}
  /// }
  /// ```
  pub fn calculate(&mut self, ego: NodeId, num_walks: usize) -> Result<(), MeritRankError> {
    if !self.graph.contains_node(ego) {
      return Err(MeritRankError::NodeDoesNotExist);
    }

    self.walks.drop_walks_from_node(ego);

    let negs = self
      .neighbors_weighted(ego, Neighbors::Negative)
      .unwrap_or(IntMap::default());

    self.personal_hits.insert(ego, Counter::new());

    for _ in 0..num_walks {
      let new_walk_id = self.walks.get_next_free_walkid();
      //let mut walk = self.walks.get_walk_mut(new_walk_id).unwrap();

      self.perform_walk(new_walk_id, ego);
      let walk = self.walks.get_walk_mut(new_walk_id).unwrap();
      let walk_steps = walk.iter().cloned();

      self.personal_hits
        .entry(ego)
        .and_modify(|counter| counter.increment_unique_counts(walk_steps));

      update_negative_hits(&mut self.neg_hits, &walk, &negs, false);
      self.walks.add_walk_to_bookkeeping(new_walk_id, 0)
    }

    Ok(())
  }


  /// Retrieves the MeritRank score for a target node from the perspective of the ego node.
  ///
  /// The score is calculated based on the accumulated hits and penalized by negative hits.
  ///
  /// # Arguments
  ///
  /// * `ego` - The ego node from whose perspective the score is calculated.
  /// * `target` - The target node for which the score is calculated.
  ///
  /// # Returns
  ///
  /// * `Weight` - The MeritRank score for the target node.
  ///
  /// # Panics
  ///
  /// A panic occurs if hits are greater than 0 but there is no path from the ego to the target node.
  ///
  /// # Example
  ///
  /// ```rust
  /// use meritrank::{MeritRank, MeritRankError, Graph, NodeId, Weight};
  ///
  /// let graph = Graph::<()>::new();
  /// let mut merit_rank = MeritRank::new(graph).unwrap();
  ///
  /// let ego : NodeId = 1;
  /// let target : NodeId = 2;
  ///
  /// let score = merit_rank.get_node_score(ego, target);
  ///
  /// println!("MeritRank score for node {:?} from node {:?}: {:?}", target, ego, score);
  /// ```
  pub fn get_node_score(&self, ego : NodeId, target : NodeId) -> Result<Weight, MeritRankError> {
    let counter = self
      .personal_hits
      .get(&ego)
      .ok_or(MeritRankError::NodeIsNotCalculated)?;

    let hits = counter.get_count(&target).copied().unwrap_or(0.0);

    if ASSERT {
      let has_path = self.graph.is_connecting(ego, target);

      if hits > 0.0 && !has_path {
        return Err(MeritRankError::NoPathExists);
      }
    }

    let binding = IntMap::default();
    let neg_hits = self.neg_hits.get(&ego).unwrap_or(&binding);
    let hits_penalized = hits + neg_hits.get(&target).copied().unwrap_or(0.0);

    Ok(hits_penalized / counter.total_count())
  }

  pub fn get_node_data(&self, ego : NodeId) -> Result<NodeData, MeritRankError> {
    match self.graph.get_node_info(ego) {
      Some((_, data)) => Ok(data),
      None            => Err(MeritRankError::NodeDoesNotExist),
    }
  }

  /// Returns the ranks of peers for the given ego node.
  ///
  /// This method calculates the ranks of peers for the specified ego node based on their node scores.
  /// It retrieves the node scores from the personal hits counter and sorts them in descending order.
  /// The ranks are limited to the specified limit if provided.
  ///
  /// # Arguments
  ///
  /// * `ego` - The ego node for which to retrieve the ranks.
  /// * `limit` - The maximum number of ranks to return (optional).
  ///
  /// # Returns
  ///
  /// A dictionary of peer ranks, where keys are peer node IDs and values are their corresponding ranks.
  pub fn get_ranks(
    &self,
    ego: NodeId,
    limit: Option<usize>,
  ) -> Result<Vec<(NodeId, Weight)>, MeritRankError> {
    let counter = self
      .personal_hits
      .get(&ego)
      .ok_or(MeritRankError::NodeDoesNotExist)?;

    let mut peer_scores: Vec<(NodeId, Weight)> = counter
      .keys()
      .iter()
      .map(|&peer| Ok((peer, self.get_node_score(ego, peer)?)))
      .collect::<Result<Vec<(NodeId, Weight)>, MeritRankError>>()?;

    peer_scores.sort_unstable_by(|(_, score1), (_, score2)| {
      score2
        .partial_cmp(score1)
        .unwrap_or(std::cmp::Ordering::Equal)
    });

    let limit = limit.unwrap_or(peer_scores.len());
    let peer_scores: Vec<(NodeId, Weight)> = peer_scores.into_iter().take(limit).collect();

    Ok(peer_scores)
  }

  /// Performs a random walk starting from the specified node.
  ///
  /// This method generates a random walk starting from the `start_node` by iteratively selecting neighbors
  /// based on their weights until the stopping condition is met.
  ///
  /// # Arguments
  ///
  /// * `start_node` - The starting node for the random walk.
  ///
  /// # Returns
  ///
  /// A `Result` containing the random walk as a `RandomWalk` if successful, or a `MeritRankError` if an error occurs.
  ///
  /// # Examples
  ///
  /// ```
  /// use meritrank::{Graph, NodeId, MeritRankError, MeritRank};
  ///
  /// let graph = Graph::<()>::new();
  /// let merit_rank = MeritRank::new(graph).unwrap();
  ///
  /// let start_node : NodeId = 1;
  ///
  /// match merit_rank.perform_walk(start_node) {
  ///   Ok(random_walk) => {
  ///     println!("Random walk: {:?}", random_walk);
  ///   }
  ///   Err(error) => {
  ///     println!("Error performing random walk: {}", error);
  ///   }
  /// }
  /// ```
  pub fn perform_walk(&mut self, walk_id: WalkId, start_node: NodeId){
    let new_segment = self.generate_walk_segment(start_node, false).unwrap();
    let mut walk = self.walks.get_walk_mut(walk_id).unwrap();
    assert_eq!(walk.len(), 0); // If we are overwriting exising walk, something went very wrong
    walk.push(start_node);
    walk.extend(&new_segment);
  }


  /// Generates a walk segment for the specified start node.
  ///
  /// This method generates a walk segment by iteratively selecting neighbors based on their weights
  /// until the stopping condition is met.
  ///
  /// # Arguments
  ///
  /// * `start_node` - The starting node for the walk segment.
  /// * `skip_alpha_on_first_step` - A boolean flag indicating whether to skip the alpha probability check
  ///   on the first step of the walk segment.
  ///
  /// # Returns
  ///
  /// A `Result` containing the walk segment as a `Vec<NodeId>` if successful, or a `MeritRankError` if an error occurs.
  ///
  /// # Examples
  ///
  /// ```
  /// use meritrank::{Graph, NodeId, MeritRankError, MeritRank};
  ///
  /// let graph = Graph::<()>::new();
  /// let merit_rank = MeritRank::new(graph).unwrap();
  ///
  /// let start_node : NodeId = 1;
  /// let skip_alpha_on_first_step = false;
  ///
  /// match merit_rank.generate_walk_segment(start_node, skip_alpha_on_first_step) {
  ///   Ok(walk_segment) => {
  ///     println!("Walk segment: {:?}", walk_segment);
  ///   }
  ///   Err(error) => {
  ///     println!("Error generating walk segment: {}", error);
  ///   }
  /// }
  /// ```
  pub fn generate_walk_segment(
    &self,
    start_node: NodeId,
    skip_alpha_on_first_step: bool,
  ) -> Result<Vec<NodeId>, MeritRankError> {
    let mut node = start_node;
    let mut segment = Vec::new();
    let mut rng = thread_rng();
    let mut skip_alpha_on_first_step = skip_alpha_on_first_step;

    while let Some(neighbors) = self.neighbors_weighted(node, Neighbors::Positive) {
      if skip_alpha_on_first_step || rng.gen::<f64>() <= self.alpha {
        skip_alpha_on_first_step = false;
        let (peers, weights): (Vec<_>, Vec<_>) = neighbors.iter().unzip();
        let next_step = Self::random_choice(&peers, &weights, &mut rng)
          .ok_or(MeritRankError::RandomChoiceError)?;
        segment.push(next_step);
        node = next_step;
      } else {
        break;
      }
    }
    Ok(segment)
  }

  /// Randomly selects an item from a list of values based on their weights.
  ///
  /// This function performs a weighted random selection by assigning probabilities to each item based on their weights,
  /// and then selecting one item at random according to those probabilities.
  ///
  /// # Arguments
  ///
  /// * `values` - A slice containing the values to select from.
  /// * `weights` - A slice containing the weights corresponding to the values.
  /// * `rng` - A mutable reference to the random number generator.
  ///
  /// # Returns
  ///
  /// An `Option` containing the selected item if successful, or `None` if the selection fails.
  pub fn random_choice<T: Copy>(values: &[T], weights: &[f64], rng: &mut impl Rng) -> Option<T> {
    let dist = WeightedIndex::new(weights).ok()?;
    let index = dist.sample(rng);
    values.get(index).copied()
  }

  /// Retrieves the weight of an edge between two nodes.
  ///
  /// This method returns the weight of the edge between the source node (`src`) and the destination node (`dest`).
  /// If no edge exists between the nodes, `None` is returned.
  ///
  /// # Arguments
  ///
  /// * `src` - The source node ID.
  /// * `dest` - The destination node ID.
  ///
  /// # Returns
  ///
  /// The weight of the edge between the source and destination nodes, or `None` if no edge exists.
  pub fn get_edge(&self, src: NodeId, dest: NodeId) -> Option<Weight> {
    self.graph.edge_weight(src, dest)
  }

  /// Updates penalties and negative hits for a specific edge.
  ///
  /// This method updates the penalties and negative hits for the edge between the source node (`src`) and the destination node (`dest`).
  /// It retrieves all walks that pass through the destination node and start with the source node.
  /// It then calculates the penalties for each affected walk based on the edge weight, and updates the negative hits accordingly.
  /// If `remove_penalties` is set to `true`, the penalties are subtracted instead of added to the negative hits.
  ///
  /// # Arguments
  ///
  /// * `src` - The source node ID.
  /// * `dest` - The destination node ID.
  /// * `remove_penalties` - A flag indicating whether to remove the penalties instead of adding them. Default is `false`.
  pub fn update_penalties_for_edge(&mut self, src: NodeId, dest: NodeId, remove_penalties: bool) {

    // Check if the edge exists and retrieve the edge weight
    // It should panic if the edge doesn't exist
    let weight = self.get_edge(src, dest).unwrap();

    let empty_map = IntMap::default();

    // Retrieve all walks that pass through the destination node and start with the source node
    let affected_walks = self.walks
    .get_visits_through_node(dest)
          .unwrap_or(&empty_map) // Provide a reference to a new empty IndexMap if None
        .iter()
    .filter_map(|(&id, &_)| {
        let walk = self.walks.get_walk(id)?;
        if walk.nodes[0] == src {
            Some(walk)
        } else {
            None
        }
    });

    // Update penalties and negative hits for each affected walk
    let ego_neg_hits = self.neg_hits.entry(src).or_insert_with(IntMap::default);

    // Create a hashmap with the negative weight of the edge for the affected node
    let neg_weights: IntMap<NodeId, Weight> = [(dest, weight)].iter().cloned().collect();

    for walk in affected_walks{
      // Calculate penalties for the affected walk
      let penalties = walk.calculate_penalties(&neg_weights);

      // Update negative hits for each node in the penalties
      for (node, penalty) in penalties {
        let adjusted_penalty = if remove_penalties { -penalty } else { penalty };

        ego_neg_hits
          .entry(node)
          .and_modify(|entry| *entry += adjusted_penalty)
          .or_insert(adjusted_penalty);
      }
    }
  }


  /// Recalculates an invalidated random walk by extending it with a new segment.
  ///
  /// This method extends an invalidated random walk (`walk`) by generating a new segment and appending it to the walk.
  /// The new segment is generated starting from the last node of the walk unless `force_first_step` is specified.
  /// If `force_first_step` is provided, it determines the first step of the new segment.
  /// The `skip_alpha_on_first_step` flag indicates whether to skip the alpha probability check on the first step of the new segment.
  ///
  /// # Arguments
  ///
  /// * `walk` - A mutable reference to the random walk that needs to be recalculated.
  /// * `force_first_step` - An optional `NodeId` representing the node to be used as the first step of the new segment.
  ///            If `None`, the last node of the walk is used as the first step.
  /// * `skip_alpha_on_first_step` - A boolean flag indicating whether to skip the alpha probability check
  ///                on the first step of the new segment.
  ///
  /// # Examples
  ///
  /// ```
  /// use meritrank::{MeritRank, Graph, NodeId, RandomWalk};
  ///
  /// let graph = Graph::<()>::new();
  /// let mut merit_rank = MeritRank::new(graph).unwrap();
  /// let mut random_walk = RandomWalk::new();
  /// random_walk.extend(&*vec![ 1, 2, ]);
  /// // ... Initialize random_walk ...
  /// let force_first_step = Some(3);
  /// let skip_alpha_on_first_step = true;
  /// merit_rank.recalc_invalidated_walk(&mut random_walk, force_first_step, skip_alpha_on_first_step);
  /// ```
  pub fn recalc_invalidated_walk(
    &mut self,
    walk_id: &WalkId,
    force_first_step: Option<NodeId>,
    mut skip_alpha_on_first_step: bool,
  ) -> Result<(), MeritRankError> {

    // Get the index where the new segment starts
    let new_segment_start = self.walks.get_walk_mut(*walk_id).unwrap().len();

    // Determine the first step based on the `force_first_step` parameter
    let first_step = match force_first_step {
      Some(step) => step,
      None => self.walks.get_walk_mut(*walk_id).unwrap().last_node().ok_or(MeritRankError::InvalidWalkLength)?,
    };

    // Check if the alpha probability should be skipped on the first step
    if force_first_step.is_some() {
      if skip_alpha_on_first_step {
        skip_alpha_on_first_step = false;
      } else {
        // Check if the random value exceeds the alpha probability
        if random::<f64>() >= self.alpha {
          return Ok(()); // Exit the function early if the alpha check fails
        }
      }
    }

    // Generate the new segment
    let mut new_segment = self.generate_walk_segment(first_step, skip_alpha_on_first_step)?;

    // Insert the first step at the beginning of the new segment if necessary
    if let Some(force_first_step) = force_first_step {
      new_segment.insert(0, force_first_step);
    }

    let walk = self.walks.get_walk_mut(*walk_id).unwrap();
    // Get the ID of the first node in the walk
    let ego = walk.first_node().ok_or(MeritRankError::InvalidWalkLength)?;
    // Update the personal hits counter for the new segment
    let counter: &mut Counter = self.personal_hits.entry(ego).or_insert_with(Counter::new);
    let diff = SetUsize::from_iter(new_segment.iter().cloned()) - &SetUsize::from_iter(walk.get_nodes().iter().cloned());
    counter.increment_unique_counts(diff.iter());

    // Extend the walk with the new segment
    walk.extend(&new_segment);

    // Add the updated walk to the collection of walks
    self.walks.add_walk_to_bookkeeping(*walk_id, new_segment_start);

    Ok(())
  }

  pub fn add_node(&mut self, node : NodeId, data : NodeData) {
    self.graph.add_node(node, data);
  }

  /// Adds an edge between two nodes with the specified weight.
  ///
  /// This method adds an edge between the source and destination nodes with the given weight.
  /// It handles various cases based on the old and new weights of the edge.
  ///
  /// # Arguments
  ///
  /// * `src` - The source node ID.
  /// * `dest` - The destination node ID.
  /// * `weight` - The weight of the edge (default is 1.0).
  ///
  /// # Panics
  ///
  /// This method panics if the source and destination nodes are the same.
  pub fn add_edge(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    if src == dest {
      panic!("Self reference not allowed");
    }

    let old_weight = self.graph.edge_weight(src, dest).unwrap_or(0.0);

    if old_weight == weight {
      return;
    }

    let old_sign = sign(old_weight);
    let new_sign = sign(weight);

    let row = old_sign as i32;
    let column = new_sign as i32;

    match (row, column) {
      (0, 0) => self.zz(src, dest, weight),
      (0, 1) => self.zp(src, dest, weight),
      (0, -1) => self.zn(src, dest, weight),
      (1, 0) => self.pz(src, dest, weight),
      (1, 1) => self.pp(src, dest, weight),
      (1, -1) => self.pn(src, dest, weight),
      (-1, 0) => self.nz(src, dest, weight),
      (-1, 1) => self.np(src, dest, weight),
      (-1, -1) => self.nn(src, dest, weight),
      _ => panic!("Invalid weight combination"),
    }
  }

  /// No-op function. Does nothing.
  fn zz(&mut self, _src: NodeId, _dest: NodeId, _weight: f64) {
    // No operation - do nothing
    // It should never happen that the old weight is zero and the new weight is zero
  }

  /// Handles the case where the old weight is zero and the new weight is positive.
  fn zp(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    assert!(weight>=0.0);
    // Clear the penalties resulting from the invalidated walks
    let step_recalc_probability =
      if OPTIMIZE_INVALIDATION && weight > EPSILON && self.graph.contains_node(src) {
        let g_edges = self
          .neighbors_weighted(src, Neighbors::Positive)
          .unwrap_or_else(IntMap::default);
        let sum_of_weights: f64 = g_edges.values().sum();
        weight / (sum_of_weights + weight)
      } else {
        0.0
      };

    let invalidated_walks_ids =
      self.walks
        .invalidate_walks_through_node(src, Some(dest), step_recalc_probability);
    // ACHTUNG! Don't mess the cut position vs the node position. Cut position = node pos + 1

    let mut negs_cache: IntMap<NodeId, IntMap<NodeId, f64>> = IntMap::default();

    for (uid, visit_pos) in &invalidated_walks_ids {
      let walk = self.walks.get_walk(*uid).unwrap();
      let negs = negs_cache
        .entry(walk.first_node().unwrap())
        .or_insert_with(|| {
          self.neighbors_weighted(walk.first_node().unwrap(), Neighbors::Negative)
            .unwrap_or_else(IntMap::default)
        });
      let cut_position = *visit_pos + 1;
      revert_counters_for_walk_from_pos(&mut self.personal_hits, walk, cut_position);

      if negs.len() > 0 {
        update_negative_hits(&mut self.neg_hits, &walk, &negs, true);
      }
    }

    if weight <= EPSILON {
      if self.graph.contains_edge(src, dest) {
        self.graph.remove_edge(src, dest);
      }
    } else {
      self.graph.add_edge(src, dest, weight);
    }

    for (walk_id, visit_pos) in &invalidated_walks_ids {
      let cut_position = visit_pos + 1;
      self.walks.remove_walk_segment_from_bookkeeping(walk_id, cut_position);
      let force_first_step = if step_recalc_probability > 0.0 {
        Some(dest)
      } else {
        None
      };

      let _ = self.recalc_invalidated_walk(
        walk_id,
        force_first_step,
        OPTIMIZE_INVALIDATION && weight <= EPSILON,
      );
      let walk_updated = self.walks.get_walk(*walk_id).unwrap();
      // self.update_negative_hits(self.walks.get_walk_mut(*walk_id).unwrap(), &mut negs_cache[&first_node], false);
      let first_node = walk_updated.first_node().unwrap();
      if let Some(negs) = negs_cache.get(&first_node) {
        if negs.len() > 0 {
          update_negative_hits(&mut self.neg_hits, walk_updated, negs, false);
        }
      } else {
        // Handle the case where negs is not found
        panic!("Negs not found");
      }

      // self.update_negative_hits(walk_mut, &mut negs_cache[&first_node], false);
    }

    if ASSERT {
      self.walks.assert_visits_consistency();
      self.assert_counters_consistency_after_edge_addition(weight);
    }
  }

  pub fn print_walks(&self) {
    self.walks.print_walks();
  }

  fn assert_counters_consistency_after_edge_addition(&self, weight: f64) {
    for (ego, hits) in &self.personal_hits {
      for (peer, count) in hits {
        let visits = self.walks.get_visits_through_node(*peer).unwrap();

        let walks: Vec<(&WalkId, &WalkId)> = visits.iter().filter(|&(walkid, _)| {
          self.walks.get_walk(*walkid).unwrap().get_nodes().first().map_or(false, |first_element| first_element == ego)
        }).collect();

        assert_eq!(walks.len(), *count as usize);
        if *count > 0.0 && weight > EPSILON && !self.graph.is_connecting(*ego, *peer) {
          assert!(false);
        }
      }
    }
  }

  /// Handles the case where the old weight is zero and the new weight is negative.
  fn zn(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    // Add an edge with the given weight
    self.graph.add_edge(src, dest, weight);
    // Update penalties for the edge
    self.update_penalties_for_edge(src, dest, false);
  }

  /// Handles the case where the old weight is positive and the new weight is zero.
  fn pz(&mut self, src: NodeId, dest: NodeId, _weight: f64) {
    // Call the zp method with weight set to 0.0
    self.zp(src, dest, 0.0);
  }

  /// Handles the case where the old weight is positive and the new weight is positive.
  fn pp(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    // Call the zp method with the given arguments
    self.zp(src, dest, weight);
  }

  /// Sets the weight of an edge to zero and updates the penalties.
  /// Then adds an edge with the given weight and updates the penalties.
  fn pn(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    // Call the pz and zn methods with the given arguments
    self.pz(src, dest, weight);
    self.zn(src, dest, weight);
  }

  /// Handles the case where the old weight is negative and the new weight is zero.
  fn nz(&mut self, src: NodeId, dest: NodeId, _weight: f64) {
    // Clear invalidated walks and update penalties
    self.update_penalties_for_edge(src, dest, true);
    // Remove the edge from the graph
    self.graph.remove_edge(src, dest);
  }

  /// Handles the case where the old weight is negative and the new weight is positive.
  fn np(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    // Call the nz and zp methods with the given arguments
    self.nz(src, dest, weight);
    self.zp(src, dest, weight);
  }

  /// Handles the case where the old weight is negative and the new weight is negative.
  fn nn(&mut self, src: NodeId, dest: NodeId, weight: f64) {
    // Call the nz and zn methods with the given arguments
    self.nz(src, dest, weight);
    self.zn(src, dest, weight);
  }

  // Experimental
  pub fn get_personal_hits(&self) -> &IntMap<NodeId, Counter> {
    &self.personal_hits
  }
}
