use crate::Ordering;
use meritrank_core::Weight;
use crate::aug_graph::ScoreResult;
use crate::log_command;

impl AugMultiGraph {
  pub fn read_scores(
    &mut self,
    context: &str,
    ego: &str,
    score_options: &ScoreOptions,
  ) -> Vec<ScoreResult> {
    log_command!("{:?} {:?} {:?}",context,ego,score_options);
    let ego_id = self.nodes.get_id(ego);
    let scores = self.fetch_all_scores(ego_id);
    self.apply_filters_and_pagination(scores, ego_id, score_options, false)
  }
  
  
  
  
  
  
  
  
  
  
  
  
}
