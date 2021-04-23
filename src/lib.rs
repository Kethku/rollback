use core::fmt::Debug;
use std::collections::HashMap;
use std::error;
use std::fmt;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, RollbackError>;

#[derive(Debug, Clone)]
pub enum RollbackError {
    InputTooOld {
        input_frame: usize,
        oldest_valid_frame: usize
    }
}

impl fmt::Display for RollbackError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RollbackError::InputTooOld { input_frame, oldest_valid_frame } => {
                write!(f, "Input for frame {} is older than oldest valid frame of {}", input_frame, oldest_valid_frame)
            }
        }
    }
}

impl error::Error for RollbackError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        // Generic error, underlying cause isn't tracked
        None
    }
}

pub struct RollbackStateManager<Input: Eq + Clone + Debug, State: Clone + Debug> {
    pub max_history: usize,
    pub oldest_frame_index: usize,
    pub current_frame_index: usize,
    pub newest_frame_index: usize,
    pub stored_state: State,
    pub current_frame_state: State,
    pub recorded_inputs: HashMap<usize, HashMap<Uuid, Input>>
}

impl<Input: Eq + Clone + Debug, State: Clone + Debug> RollbackStateManager<Input, State> {
    pub fn new(initial_state: State, max_rollback: usize) -> RollbackStateManager<Input, State> {
        RollbackStateManager {
            max_history: max_rollback,
            oldest_frame_index: 0,
            current_frame_index: 0,
            newest_frame_index: 0,
            stored_state: initial_state.clone(),
            current_frame_state: initial_state,
            recorded_inputs: HashMap::new()
        }
    }

    // Builds inputs up by looping backwards searching for inputs for each id
    pub fn get_frame_inputs(&self, index: usize) -> HashMap<Uuid, Input> {
        let mut inputs = HashMap::new();

        for previous_index in (self.oldest_frame_index..index + 1).rev() {
            if let Some(current_frame_inputs) = self.recorded_inputs.get(&previous_index) {
                for (id, input) in current_frame_inputs.iter() {
                    if !inputs.contains_key(id) {
                        inputs.insert(id.clone(), input.clone());
                    }
                }
            }
        }
        inputs
    }
 
    // Show the current frame state
    fn compute_frame_state<F>(&self, index: usize, update: F) -> State 
            where F: Fn(&HashMap<Uuid, Input>, State) -> State {
        // Clone the stored frame and update it until the current frame
        let mut state = self.stored_state.clone();
        for i in self.oldest_frame_index .. index + 1 {
            state = update(&self.get_frame_inputs(i), state);
        }
        state
    }

    // Progress the frame counter by 1 and return the state of that frame under current known
    // inputs
    pub fn progress_frame<F>(&mut self, update: F) where F: Fn(&HashMap<Uuid, Input>, State) -> State {
        // Increment current frame
        self.current_frame_index = self.current_frame_index + 1;
        // Compute oldest possible frame
        let max_oldest_frame = self.current_frame_index.checked_sub(self.max_history).unwrap_or(0);
        // If the currently recorded oldest frame is older than the oldest possible frame, update
        // the stored state until the oldest recorded frame matches the oldest possible frame
        if self.oldest_frame_index < max_oldest_frame {
            let mut state = self.stored_state.clone();
            for i in 0..max_oldest_frame - self.oldest_frame_index {
                let frame = self.oldest_frame_index + i;
                state = update(&self.get_frame_inputs(frame), state);
            }

            self.recorded_inputs.insert(max_oldest_frame, self.get_frame_inputs(max_oldest_frame));
            self.oldest_frame_index = max_oldest_frame;
            self.stored_state = state;
        }

        // Update the stored frame till the current frame index and return
        self.current_frame_state = self.compute_frame_state(self.current_frame_index, update);
    }

    // Store input or a given player id
    pub fn handle_input(&mut self, frame: usize, id: Uuid, input: Input) -> Result<()> {
        if frame < self.oldest_frame_index {
            return Err(RollbackError::InputTooOld {
                input_frame: frame,
                oldest_valid_frame: self.oldest_frame_index
            })
        }

        let recorded_inputs = self.recorded_inputs.entry(frame).or_insert(HashMap::new());
        recorded_inputs.insert(id, input);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Input = u64;
    type State = u64;

    lazy_static! {
        static ref P1ID: Uuid = Uuid::new_v4();
        static ref P2ID: Uuid = Uuid::new_v4();
    }
    
    fn update(input: &HashMap<Uuid, Input>, state: State) -> State {
        let mut current_state = state;

        for input_value in input.values() {
            current_state = current_state + input_value;
        }

        current_state
    }

    #[test]
    fn HandleInput_GetFrameInput_MultipleFrames_BuildsInputs() -> Result<()> {
        let mut rollback_manager = RollbackStateManager::new(0, 4);

        rollback_manager.handle_input(0, P1ID.clone(), 1)?;
        rollback_manager.handle_input(1, P2ID.clone(), 2)?;
        rollback_manager.handle_input(2, P1ID.clone(), 0)?;
        rollback_manager.handle_input(2, P2ID.clone(), 0)?;

        let frame_0_inputs = rollback_manager.get_frame_inputs(0);
        assert_eq!(frame_0_inputs.get(&P1ID.clone()), Some(&1));
        assert_eq!(frame_0_inputs.get(&P2ID.clone()), None);

        let frame_0_inputs = rollback_manager.get_frame_inputs(1);
        assert_eq!(frame_0_inputs.get(&P1ID.clone()), Some(&1));
        assert_eq!(frame_0_inputs.get(&P2ID.clone()), Some(&2));

        let frame_0_inputs = rollback_manager.get_frame_inputs(2);
        assert_eq!(frame_0_inputs.get(&P1ID.clone()), Some(&0));
        assert_eq!(frame_0_inputs.get(&P2ID.clone()), Some(&0));

        Ok(())
    }

    #[test]
    fn ProgressFrame_ComputesCorrectState() -> Result<()> {
        let mut rollback_manager = RollbackStateManager::new(1, 4);

        rollback_manager.handle_input(1, P1ID.clone(), 1)?;
        rollback_manager.handle_input(2, P2ID.clone(), 2)?;
        rollback_manager.handle_input(3, P1ID.clone(), 0)?;
        rollback_manager.handle_input(3, P2ID.clone(), 0)?;

        // frame 1 update
        // 1 + (1 + 0) = 2
        // frame 2 update
        // 2 + (1 + 2) = 5

        assert_eq!(rollback_manager.current_frame_index, 0);
        assert_eq!(rollback_manager.current_frame_state, 1);
        
        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 1);
        assert_eq!(rollback_manager.current_frame_state, 2);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 2);
        assert_eq!(rollback_manager.current_frame_state, 5);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 3);
        assert_eq!(rollback_manager.current_frame_state, 5);

        Ok(())
    }

    #[test]
    fn ProgressFrame_PastOldestFrame_PreservesInput() -> Result<()> {
        let mut rollback_manager = RollbackStateManager::new(0, 3);

        rollback_manager.handle_input(1, P1ID.clone(), 1)?;
        rollback_manager.handle_input(3, P1ID.clone(), 0)?;

        assert_eq!(rollback_manager.get_frame_inputs(1).get(&P1ID), Some(&1));
        assert_eq!(rollback_manager.get_frame_inputs(2).get(&P1ID), Some(&1));
        assert_eq!(rollback_manager.get_frame_inputs(3).get(&P1ID), Some(&0));
        assert_eq!(rollback_manager.get_frame_inputs(4).get(&P1ID), Some(&0));

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 1);
        assert_eq!(rollback_manager.current_frame_state, 1);
        assert_eq!(rollback_manager.oldest_frame_index, 0);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 2);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 0);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 3);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 0);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 4);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 1);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 5);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 2);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 6);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 3);

        rollback_manager.progress_frame(update);
        assert_eq!(rollback_manager.current_frame_index, 7);
        assert_eq!(rollback_manager.current_frame_state, 2);
        assert_eq!(rollback_manager.oldest_frame_index, 4);

        Ok(())
    }
}
