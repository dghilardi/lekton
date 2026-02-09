use markdown::{to_mdast, ParseOptions};
use markdown::mdast::{Node, Link};

pub struct LinkValidator;

impl LinkValidator {
    pub fn extract_links(content: &str) -> Vec<String> {
        let options = ParseOptions::gfm();
        if let Ok(ast) = to_mdast(content, &options) {
            Self::find_links(&ast)
        } else {
            vec![]
        }
    }

    fn find_links(node: &Node) -> Vec<String> {
        let mut links = vec![];
        match node {
            Node::Link(Link { url, .. }) => {
                if url.starts_with("/doc/") {
                    links.push(url.trim_start_matches("/doc/").to_string());
                }
            }
            _ => {
                if let Some(children) = node.children() {
                    for child in children {
                        links.extend(Self::find_links(child));
                    }
                }
            }
        }
        links
    }
}
