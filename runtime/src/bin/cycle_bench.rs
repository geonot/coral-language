use runtime::{
    ListObject, ValueHandle, collect_cycles, coral_make_list, coral_make_number,
    coral_value_release, coral_value_retain, cycle_stats, possible_root, reset_cycle_detector,
};
use std::time::Instant;

unsafe fn create_simple_cycle() -> ValueHandle {
    let list = coral_make_list(std::ptr::null(), 0);
    let list_obj = &mut *((*list).payload.ptr as *mut ListObject);
    list_obj.items.push(list);
    coral_value_retain(list);
    list
}

unsafe fn create_chain_cycle(length: usize) -> Vec<ValueHandle> {
    let mut nodes = Vec::with_capacity(length);

    for _ in 0..length {
        nodes.push(coral_make_list(std::ptr::null(), 0));
    }

    for i in 0..length {
        let next_idx = (i + 1) % length;
        let current = &mut *((*nodes[i]).payload.ptr as *mut runtime::ListObject);
        current.items.push(nodes[next_idx]);
        coral_value_retain(nodes[next_idx]);
    }

    nodes
}

fn benchmark_no_cycles(iterations: usize) -> std::time::Duration {
    reset_cycle_detector();

    let start = Instant::now();

    for i in 0..iterations {
        unsafe {
            let num = coral_make_number(i as f64);
            let list = coral_make_list(&num as *const _, 1);

            possible_root(list);

            coral_value_release(list);
            coral_value_release(num);
        }
    }

    start.elapsed()
}

fn benchmark_with_cycles(num_cycles: usize) -> std::time::Duration {
    reset_cycle_detector();

    let start = Instant::now();

    let mut cycles = Vec::new();
    for _ in 0..num_cycles {
        unsafe {
            let cycle = create_simple_cycle();
            cycles.push(cycle);
            possible_root(cycle);
        }
    }

    collect_cycles();

    let elapsed = start.elapsed();

    for cycle in cycles {
        unsafe {
            coral_value_release(cycle);
        }
    }

    elapsed
}

fn benchmark_large_cycle_collection() -> std::time::Duration {
    reset_cycle_detector();

    unsafe {
        let mut all_cycles = Vec::new();

        for chain_length in 2..=10 {
            for _ in 0..10 {
                let chain = create_chain_cycle(chain_length);
                for &node in &chain {
                    possible_root(node);
                }
                all_cycles.push(chain);
            }
        }

        let start = Instant::now();
        collect_cycles();
        let elapsed = start.elapsed();

        for chain in all_cycles {
            for node in chain {
                coral_value_release(node);
            }
        }

        elapsed
    }
}

fn main() {
    println!("Cycle Collection Benchmark");
    println!("==========================");

    let no_cycle_time = benchmark_no_cycles(10000);
    println!("10,000 operations without cycles: {:?}", no_cycle_time);

    let few_cycles_time = benchmark_with_cycles(10);
    println!("10 simple cycles detection: {:?}", few_cycles_time);

    let many_cycles_time = benchmark_with_cycles(100);
    println!("100 simple cycles detection: {:?}", many_cycles_time);

    let large_collection_time = benchmark_large_cycle_collection();
    println!("Large mixed cycle collection: {:?}", large_collection_time);

    let baseline_per_op = no_cycle_time.as_nanos() / 10000;
    let cycle_overhead_10 = few_cycles_time.as_nanos() / 10;
    let cycle_overhead_100 = many_cycles_time.as_nanos() / 100;

    println!("\nPerformance Analysis:");
    println!("Baseline per operation: {} ns", baseline_per_op);
    println!("Cycle detection per cycle (10): {} ns", cycle_overhead_10);
    println!("Cycle detection per cycle (100): {} ns", cycle_overhead_100);
    println!(
        "Large collection took: {} ms",
        large_collection_time.as_millis()
    );

    let overhead_percent = if baseline_per_op > 0 {
        ((cycle_overhead_10 as f64 / baseline_per_op as f64) - 1.0) * 100.0
    } else {
        0.0
    };

    println!("\nOverhead analysis:");
    if overhead_percent <= 5.0 {
        println!(
            "✓ Cycle detection overhead: {:.2}% (target: <5%)",
            overhead_percent
        );
    } else {
        println!(
            "✗ Cycle detection overhead: {:.2}% (exceeds 5% target)",
            overhead_percent
        );
    }

    let (cycles_detected, values_collected) = cycle_stats();
    println!("\nCycle Collection Statistics:");
    println!("Total cycles detected: {}", cycles_detected);
    println!("Total values collected: {}", values_collected);
}
