use super::types::SearchState;

#[inline(always)]
pub(super) fn frontier_heap_push<Node: Copy + Ord, Key: Copy + Ord>(
    heap: &mut Vec<SearchState<Node, Key>>,
    state: SearchState<Node, Key>,
) {
    heap.push(state);
    let len = heap.len();
    frontier_sift_up(heap, len);
}

#[inline(always)]
pub(super) fn frontier_heap_pop<Node: Copy + Ord, Key: Copy + Ord>(
    heap: &mut Vec<SearchState<Node, Key>>,
) -> Option<SearchState<Node, Key>> {
    if heap.is_empty() {
        return None;
    }
    if heap.len() == 1 {
        return heap.pop();
    }

    let len = heap.len();
    let best = heap[0];
    let hole = frontier_floyd_sift_down(heap);
    let last = len - 1;
    if hole == last {
        heap[hole] = best;
    } else {
        let value = heap[last];
        heap[hole] = value;
        frontier_sift_up_to(heap, hole + 1);
        heap[last] = best;
    }
    heap.pop()
}

#[inline(always)]
fn frontier_heap_prefers<Node: Copy + Ord, Key: Copy + Ord>(
    lhs: &SearchState<Node, Key>,
    rhs: &SearchState<Node, Key>,
) -> bool {
    lhs.priority > rhs.priority
}

#[inline(always)]
fn frontier_sift_up<Node: Copy + Ord, Key: Copy + Ord>(
    heap: &mut [SearchState<Node, Key>],
    len: usize,
) {
    frontier_sift_up_to(heap, len);
}

#[inline(always)]
fn frontier_sift_up_to<Node: Copy + Ord, Key: Copy + Ord>(
    heap: &mut [SearchState<Node, Key>],
    len: usize,
) {
    if len <= 1 {
        return;
    }
    let mut child = len - 1;
    let value = heap[child];
    let mut parent = (child - 1) / 2;
    if frontier_heap_prefers(&heap[parent], &value) {
        while {
            heap[child] = heap[parent];
            child = parent;
            if child == 0 {
                false
            } else {
                parent = (child - 1) / 2;
                frontier_heap_prefers(&heap[parent], &value)
            }
        } {}
        heap[child] = value;
    }
}

#[inline(always)]
fn frontier_floyd_sift_down<Node: Copy + Ord, Key: Copy + Ord>(
    heap: &mut [SearchState<Node, Key>],
) -> usize {
    let len = heap.len();
    debug_assert!(len >= 2);

    let mut hole = 0usize;
    let mut child = 0usize;
    loop {
        child = 2 * child + 1;
        let mut child_index = child;
        if child + 1 < len && frontier_heap_prefers(&heap[child_index], &heap[child_index + 1]) {
            child_index += 1;
            child += 1;
        }
        heap[hole] = heap[child_index];
        hole = child_index;
        if child > (len - 2) / 2 {
            break;
        }
    }
    hole
}
