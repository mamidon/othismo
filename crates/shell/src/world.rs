//! A running image as the shell sees it: built from a snapshot, it answers
//! where things are, where to look from, and how to get there.
//!
//! Every view belongs to someone: the root image, or an instance. The
//! places to start looking from, the roots, are the root image's own view
//! and every instance with a view that nothing mounts. Only a `mount` makes
//! a parent: a `bind` or an export passes on what is already held, so it
//! doesn't. The route to an instance is the shortest chain of mounts to it
//! from a root.

use image::program::Kind;
use protocol::ns::{Namespace, Resolved, Status};
use runtime::operator::{How, InstanceInfo, Link, Snapshot};
use std::collections::VecDeque;

/// Where requests are made from and paths resolve: the root image's own
/// view, or an instance's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Viewpoint {
    Root,
    Instance(usize),
}

impl Viewpoint {
    /// The instance, as [`runtime::operator::Command::Request`] takes it.
    pub fn instance(self) -> Option<usize> {
        match self {
            Viewpoint::Root => None,
            Viewpoint::Instance(i) => Some(i),
        }
    }
}

/// What an instance is, as far as looking at it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The runtime's, touching the host: a console, a listener, files.
    Host,
    /// The runtime's, touching nothing outside the image.
    Memory,
    /// A module.
    Wasm,
    /// An instance of an image: objects made of objects.
    Image,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Host => "host",
            Class::Memory => "memory",
            Class::Wasm => "wasm",
            Class::Image => "image",
        }
    }
}

/// One step of a route: an instance, and the mount point in the previous
/// step's view it was reached through (none for the root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub at: Viewpoint,
    pub via: Option<String>,
    pub read_only: bool,
}

pub struct World {
    pub snapshot: Snapshot,
    ns: Namespace,
}

impl World {
    pub fn new(snapshot: Snapshot) -> World {
        let n = snapshot.instances.len();
        let mut ns = Namespace::new(n + 1);
        for (i, inst) in snapshot.instances.iter().enumerate() {
            for l in &inst.view {
                ns.mount_resolved(i, &l.at, l.target.clone());
            }
            for l in &inst.exposed {
                ns.expose_resolved(i, &l.at, l.target.clone());
            }
        }
        for l in &snapshot.root {
            ns.mount_resolved(n, &l.at, l.target.clone());
        }
        World { snapshot, ns }
    }

    pub fn instances(&self) -> &[InstanceInfo] {
        &self.snapshot.instances
    }

    pub fn instance(&self, i: usize) -> &InstanceInfo {
        &self.snapshot.instances[i]
    }

    pub fn class(&self, i: usize) -> Class {
        match self.snapshot.instances[i].kind {
            Kind::Wasm { .. } => Class::Wasm,
            Kind::Image { .. } => Class::Image,
            Kind::Console | Kind::Tcp | Kind::Files => Class::Host,
            Kind::Packed { .. } | Kind::MemoryConsole | Kind::MemoryListener => Class::Memory,
        }
    }

    /// Whether it has a view to look from: the runtime's objects don't.
    pub fn has_view(&self, at: Viewpoint) -> bool {
        match at {
            Viewpoint::Root => true,
            Viewpoint::Instance(i) => matches!(self.class(i), Class::Wasm | Class::Image),
        }
    }

    /// Whether requests made from there carry the operator flag: an image
    /// is the operator of what it holds; an object never is.
    pub fn is_operator(&self, at: Viewpoint) -> bool {
        match at {
            Viewpoint::Root => true,
            Viewpoint::Instance(i) => self.class(i) == Class::Image,
        }
    }

    pub fn view(&self, at: Viewpoint) -> &[Link] {
        match at {
            Viewpoint::Root => &self.snapshot.root,
            Viewpoint::Instance(i) => &self.snapshot.instances[i].view,
        }
    }

    /// `/` for the root image, an instance's name otherwise.
    pub fn name(&self, at: Viewpoint) -> &str {
        match at {
            Viewpoint::Root => "/",
            Viewpoint::Instance(i) => &self.snapshot.instances[i].name,
        }
    }

    /// The viewpoint called `name`: `/` or an instance's name.
    pub fn find(&self, name: &str) -> Option<Viewpoint> {
        if name == "/" {
            return Some(Viewpoint::Root);
        }
        self.snapshot.instances.iter().position(|i| i.name == name).map(Viewpoint::Instance)
    }

    /// What `at` mounts: the instances its view holds.
    pub fn holds(&self, at: Viewpoint) -> impl Iterator<Item = &Link> {
        self.view(at).iter().filter(|l| l.how == How::Mount)
    }

    /// Who holds instance `i`, and where in their views.
    pub fn holders(&self, i: usize) -> Vec<(Viewpoint, &str)> {
        let mut out = vec![];
        let all = std::iter::once(Viewpoint::Root).chain((0..self.snapshot.instances.len()).map(Viewpoint::Instance));
        for at in all {
            for l in self.holds(at).filter(|l| l.target.inst == i) {
                out.push((at, l.at.as_str()));
            }
        }
        out
    }

    /// The places to start looking from: the root image's view, then every
    /// instance with a view that nothing mounts, in image order.
    pub fn roots(&self) -> Vec<Viewpoint> {
        let held = |i: usize| !self.holders(i).is_empty();
        let mut roots = vec![Viewpoint::Root];
        roots.extend((0..self.snapshot.instances.len()).filter(|&i| !held(i)).map(Viewpoint::Instance).filter(|&v| self.has_view(v)));
        roots
    }

    /// Everything `root` holds, at any depth, in the order a search finds it.
    pub fn descendants(&self, root: Viewpoint) -> Vec<usize> {
        let (mut seen, mut queue) = (vec![], VecDeque::from([root]));
        while let Some(at) = queue.pop_front() {
            for l in self.holds(at) {
                if !seen.contains(&l.target.inst) {
                    seen.push(l.target.inst);
                    queue.push_back(Viewpoint::Instance(l.target.inst));
                }
            }
        }
        seen
    }

    /// The shortest chain of mounts from `root` to `to`, if there is one.
    pub fn route(&self, root: Viewpoint, to: Viewpoint) -> Option<Vec<Hop>> {
        let mut prev: Vec<(Viewpoint, Option<(Viewpoint, &Link)>)> = vec![(root, None)];
        let mut queue = VecDeque::from([root]);
        while let Some(at) = queue.pop_front() {
            if at == to {
                break;
            }
            for l in self.holds(at) {
                let next = Viewpoint::Instance(l.target.inst);
                if !prev.iter().any(|(v, _)| *v == next) {
                    prev.push((next, Some((at, l))));
                    queue.push_back(next);
                }
            }
        }
        let mut hops = vec![];
        let mut at = to;
        loop {
            let (_, how) = prev.iter().find(|(v, _)| *v == at)?;
            match how {
                None => {
                    hops.push(Hop { at, via: None, read_only: false });
                    break;
                }
                Some((from, l)) => {
                    hops.push(Hop { at, via: Some(l.at.clone()), read_only: l.target.read_only });
                    at = *from;
                }
            }
        }
        hops.reverse();
        Some(hops)
    }

    /// The route to `to` to show: from `entered` if it holds it, otherwise
    /// from the first root that does, otherwise `to` alone.
    pub fn canonical(&self, entered: Viewpoint, to: Viewpoint) -> Vec<Hop> {
        self.route(entered, to)
            .or_else(|| self.roots().into_iter().find_map(|r| self.route(r, to)))
            .unwrap_or_else(|| vec![Hop { at: to, via: None, read_only: false }])
    }

    /// Where `path` in `at`'s view leads, as the runtime would resolve it.
    pub fn resolve(&self, at: Viewpoint, path: &str) -> Result<Resolved, Status> {
        let view = at.instance().unwrap_or(self.snapshot.instances.len());
        self.ns.resolve(view, path)
    }
}
