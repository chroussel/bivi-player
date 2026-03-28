/**
 * Creates a debounced function that delays invoking `fn` until `ms` milliseconds
 * after the last call. Returns the debounced function with a `.cancel()` method.
 */
export function debounce(fn, ms) {
    let timer = null;
    const debounced = (...args) => {
        clearTimeout(timer);
        timer = setTimeout(() => fn(...args), ms);
    };
    debounced.cancel = () => clearTimeout(timer);
    return debounced;
}

/**
 * Creates an accumulator that collects values via `add()`, then calls `fn(total)`
 * after `ms` of inactivity. Resets after firing.
 */
export function accumulatedDebounce(fn, ms) {
    let timer = null;
    let total = 0;
    let base = null;

    return {
        add(value, getBase) {
            if (base === null) base = getBase();
            total += value;
            clearTimeout(timer);
            timer = setTimeout(() => {
                fn(base + total);
                base = null;
                total = 0;
            }, ms);
        },
        pending() { return base !== null ? base + total : null; },
        cancel() { clearTimeout(timer); base = null; total = 0; },
    };
}
