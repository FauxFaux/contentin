use std::fmt;

use std::rc::Rc;

pub struct SList<T> {
    head: Rc<Node<T>>,
}

impl<T: Clone> SList<T> {
    pub fn head(obj: T) -> SList<T> {
        SList {
            head: Rc::new(Node {
                next: None,
                value: obj,
            }),
        }
    }

    pub fn plus(&self, obj: T) -> SList<T> {
        SList {
            head: Rc::new(Node {
                next: Some(self.head.clone()),
                value: obj,
            }),
        }
    }

    pub fn inner(&self) -> &T {
        &self.head.value
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut ret: Vec<T> = self.iter().collect();
        ret.reverse();
        ret
    }

    pub fn iter(&self) -> SListIter<T> {
        SListIter {
            next: Some(self.head.clone()),
        }
    }
}

pub struct SListIter<T> {
    next: Option<Rc<Node<T>>>,
}

impl<T: Clone> Iterator for SListIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let list = self.next.as_ref()?.clone();
        self.next = list.next.clone();
        Some(list.value.clone())
    }
}

impl<T: fmt::Display> fmt::Display for SList<T>
where
    T: Clone + fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.to_vec())
    }
}

#[derive(Debug)]
struct Node<T> {
    next: Option<Rc<Node<T>>>,
    value: T,
}
