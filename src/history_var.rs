use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct HistoryEntry<T> {
    pub value: T,
    pub setter_id: usize,
    pub timestamp: u64,
    pub reason: Option<String>,
    pub entry_id: u64, // Unique ID for each entry
}

impl<T> HistoryEntry<T> {
    fn new(value: T, setter_id: usize, reason: Option<String>, entry_id: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            value,
            setter_id,
            timestamp,
            reason,
            entry_id,
        }
    }
}

#[derive(Debug)]
pub struct HistoryVariable<T> {
    current_value: T,
    history: VecDeque<HistoryEntry<T>>,
    max_history: usize,
    next_entry_id: u64,
}

impl<T: Clone> HistoryVariable<T> {
    /// Create a new history variable with initial value and setter ID
    pub fn new(initial_value: T, setter_id: usize, max_history: usize) -> Self {
        let mut history = VecDeque::with_capacity(max_history + 1);
        history.push_back(HistoryEntry::new(
            initial_value.clone(),
            setter_id,
            Some("Initial value".to_string()),
            0,
        ));

        Self {
            current_value: initial_value,
            history,
            max_history,
            next_entry_id: 1,
        }
    }

    /// Set a new value with setter ID and optional reason
    pub fn set(&mut self, value: T, setter_id: usize, reason: Option<&str>) {
        let entry = HistoryEntry::new(
            value.clone(),
            setter_id,
            reason.map(|r| r.to_string()),
            self.next_entry_id,
        );

        self.current_value = value;
        self.history.push_back(entry);
        self.next_entry_id += 1;

        // Maintain max history size
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }
    }

    /// Get current value
    pub fn get(&self) -> &T {
        &self.current_value
    }

    /// Remove ANY change by a specific setter ID (not just the most recent)
    /// This removes the entry but preserves the chronological order of remaining entries
    pub fn remove_change_by_setter(&mut self, setter_id: usize) -> Result<T, String> {
        if self.history.len() <= 1 {
            return Err("Cannot remove: only initial value remains".to_string());
        }

        // Find the LAST change by this setter (most recent)
        let mut found_index = None;
        for (i, entry) in self.history.iter().enumerate().rev() {
            if entry.setter_id == setter_id && i > 0 {
                // Don't remove initial value
                found_index = Some(i);
                break;
            }
        }

        match found_index {
            Some(index) => {
                self.history.remove(index);

                // Update current value to the last entry in history
                if let Some(last_entry) = self.history.back() {
                    self.current_value = last_entry.value.clone();
                    Ok(self.current_value.clone())
                } else {
                    Err("History corrupted".to_string())
                }
            }
            None => Err(format!("No changes found by setter ID: {}", setter_id)),
        }
    }

    /// Remove a specific entry by entry ID
    pub fn remove_change_by_entry_id(&mut self, entry_id: u64) -> Result<T, String> {
        if entry_id == 0 {
            return Err("Cannot remove initial value".to_string());
        }

        let mut found_index = None;
        for (i, entry) in self.history.iter().enumerate() {
            if entry.entry_id == entry_id {
                found_index = Some(i);
                break;
            }
        }

        match found_index {
            Some(index) => {
                self.history.remove(index);

                // Update current value to the last entry in history
                if let Some(last_entry) = self.history.back() {
                    self.current_value = last_entry.value.clone();
                    Ok(self.current_value.clone())
                } else {
                    Err("History corrupted".to_string())
                }
            }
            None => Err(format!("No entry found with ID: {}", entry_id)),
        }
    }

    /// Remove ALL changes by a specific setter ID
    pub fn remove_all_changes_by_setter(&mut self, setter_id: usize) -> Result<usize, String> {
        let initial_len = self.history.len();

        // Keep only entries that are NOT from this setter (except initial value at index 0)
        let mut new_history = VecDeque::new();
        for (i, entry) in self.history.iter().enumerate() {
            if i == 0 || entry.setter_id != setter_id {
                new_history.push_back(entry.clone());
            }
        }

        let removed_count = self.history.len() - new_history.len();
        self.history = new_history;

        // Update current value to the last entry in history
        if let Some(last_entry) = self.history.back() {
            self.current_value = last_entry.value.clone();
        }

        if removed_count > 0 {
            Ok(removed_count)
        } else {
            Err(format!("No changes found by setter ID: {}", setter_id))
        }
    }

    /// Undo the last change (regardless of setter) - for backward compatibility
    pub fn undo_last(&mut self) -> Result<T, String> {
        if self.history.len() <= 1 {
            return Err("Cannot undo: no previous values".to_string());
        }

        self.history.pop_back();

        if let Some(last_entry) = self.history.back() {
            self.current_value = last_entry.value.clone();
            Ok(self.current_value.clone())
        } else {
            Err("History corrupted".to_string())
        }
    }

    /// Get the full history
    pub fn get_history(&self) -> &VecDeque<HistoryEntry<T>> {
        &self.history
    }

    /// Get changes by a specific setter ID
    pub fn get_changes_by_setter(&self, setter_id: usize) -> Vec<&HistoryEntry<T>> {
        self.history
            .iter()
            .filter(|entry| entry.setter_id == setter_id)
            .collect()
    }

    /// Rollback to a specific timestamp
    pub fn rollback_to_timestamp(&mut self, timestamp: u64) -> Result<T, String> {
        let mut rollback_index = None;
        for (i, entry) in self.history.iter().enumerate() {
            if entry.timestamp <= timestamp {
                rollback_index = Some(i);
            } else {
                break;
            }
        }

        match rollback_index {
            Some(index) => {
                self.history.truncate(index + 1);
                if let Some(entry) = self.history.back() {
                    self.current_value = entry.value.clone();
                    Ok(self.current_value.clone())
                } else {
                    Err("No valid entry found at timestamp".to_string())
                }
            }
            None => Err("No entry found before or at timestamp".to_string()),
        }
    }

    /// Get statistics about the variable
    pub fn get_stats(&self) -> VariableStats {
        let unique_setters: std::collections::HashSet<usize> =
            self.history.iter().map(|e| e.setter_id).collect();

        VariableStats {
            total_changes: self.history.len().saturating_sub(1),
            unique_setters: unique_setters.len(),
            first_change: self.history.front().map(|e| e.timestamp),
            last_change: self.history.back().map(|e| e.timestamp),
        }
    }
}

#[derive(Debug)]
pub struct VariableStats {
    pub total_changes: usize,
    pub unique_setters: usize,
    pub first_change: Option<u64>,
    pub last_change: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_change_preserves_order() {
        let mut var = HistoryVariable::new("initial", 0, 10);

        var.set("change1", 1001, Some("First change")); // Entry ID 1
        var.set("change2", 1002, Some("Second change")); // Entry ID 2
        var.set("change3", 1001, Some("Third change")); // Entry ID 3
        var.set("change4", 1003, Some("Fourth change")); // Entry ID 4

        // Remove user 1001's most recent change (change3)
        let result = var.remove_change_by_setter(1001);
        assert!(result.is_ok());

        // Current value should be "change4" (the last remaining entry)
        assert_eq!(*var.get(), "change4");

        // History should be: initial -> change1 -> change2 -> change4
        assert_eq!(var.get_history().len(), 4);
        assert_eq!(var.get_history()[1].value, "change1");
        assert_eq!(var.get_history()[2].value, "change2");
        assert_eq!(var.get_history()[3].value, "change4");
    }

    #[test]
    fn test_remove_specific_entry() {
        let mut var = HistoryVariable::new(100, 0, 10);

        var.set(200, 1001, None); // Entry ID 1
        var.set(300, 1002, None); // Entry ID 2
        var.set(400, 1001, None); // Entry ID 3

        // Remove specific entry (ID 2)
        let result = var.remove_change_by_entry_id(2);
        assert!(result.is_ok());

        // Current value should be 400 (last entry)
        assert_eq!(*var.get(), 400);

        // History should have: initial(100) -> entry1(200) -> entry3(400)
        assert_eq!(var.get_history().len(), 3);
    }

    #[test]
    fn test_remove_all_changes_by_setter() {
        let mut var = HistoryVariable::new("initial", 0, 10);

        var.set("change1", 1001, None);
        var.set("change2", 1002, None);
        var.set("change3", 1001, None);
        var.set("change4", 1002, None);

        // Remove all changes by user 1001
        let result = var.remove_all_changes_by_setter(1001);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2); // Removed 2 entries

        // Should have: initial -> change2 -> change4
        assert_eq!(var.get_history().len(), 3);
        assert_eq!(*var.get(), "change4");
    }
}

// Example usage
fn sample_usage() {
    let mut config_value = HistoryVariable::new("initial".to_string(), 0, 50);

    config_value.set("value_a".to_string(), 1001, Some("User 1001 change"));
    config_value.set("value_b".to_string(), 1002, Some("User 1002 change"));
    config_value.set("value_c".to_string(), 1001, Some("User 1001 again"));
    config_value.set("value_d".to_string(), 1003, Some("User 1003 change"));

    println!("Current value: {}", config_value.get());

    // Remove user 1001's most recent change (preserves order)
    if let Ok(_) = config_value.remove_change_by_setter(1001) {
        println!("After removing user 1001's change: {}", config_value.get());
    }
    // Print remaining history
    for entry in config_value.get_history() {
        println!(
            "Entry {}: User {} set '{}' ({})",
            entry.entry_id,
            entry.setter_id,
            entry.value,
            entry.reason.as_deref().unwrap_or("No reason")
        );
    }
}
