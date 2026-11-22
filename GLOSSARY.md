# Sampler

A macOS tool for capturing audio from a running application (or the whole system) and saving parts of it as samples.

## Language

**Recording**:
All audio captured into the buffer during one capture session.
_Avoid_: Take, buffer, clip

**Selection**:
A contiguous span of the Recording that Play and Save act on; with no Selection, they act on the whole Recording.
_Avoid_: Region, crop, range

**Selection handle**:
The start or end edge of a Selection, which can be dragged to resize it.
_Avoid_: Marker, edge, boundary
