use std::rc::Rc;

#[derive(Debug)]
pub struct Node<T> {
    next: Option<Rc<Node<T>>>,
    value: T,
}

impl<T: Clone> Node<T> {
    pub fn head(obj: T) -> Rc<Node<T>> {
        Rc::new(Node {
            next: None,
            value: obj,
        })
    }

    pub fn plus(what: &Rc<Node<T>>, obj: T) -> Rc<Node<T>> {
        Rc::new(Node {
            next: Some(what.clone()),
            value: obj
        })
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut ret = Vec::new();
        let mut val = self;
        loop {
            ret.push(val.value.clone());
            if let Some(ref next) = val.next {
                val = next;
            } else {
                break;
            }
        }
        ret.reverse();
        ret
    }

    pub fn inner(&self) -> &T {
        &self.value
    }
}
