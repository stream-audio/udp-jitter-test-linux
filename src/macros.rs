#[allow(unused_macros)]
macro_rules! some_or_ret {
    ( $e:expr ) => {{
        match $e {
            Some(v) => v,
            None => return,
        }
    }};
    ( $e:expr, $ret:expr ) => {{
        match $e {
            Some(v) => v,
            None => {
                return $ret;
            }
        }
    }};
}

#[allow(unused_macros)]
macro_rules! some_or_cont {
    ( $e:expr ) => {{
        match $e {
            Some(v) => v,
            None => continue,
        }
    }};
}

#[allow(unused_macros)]
macro_rules! some_or_brk {
    ( $e:expr ) => {{
        match $e {
            Some(v) => v,
            None => break,
        }
    }};
    ( $e:expr, $ret:expr ) => {{
        match $e {
            Some(v) => v,
            None => {
                break $ret;
            }
        }
    }};
}
