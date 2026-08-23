<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (CG-1: xlinked boundedBy). -->
<!-- A single Building whose lod2Solid holds four INLINE polygons p0..p3, and -->
<!-- whose bldg:boundedBy semantic surfaces attach semantics to those solid   -->
<!-- faces purely by gml:surfaceMember xlink:href (NOT inline geometry):       -->
<!-- Ground -> p0, Wall -> {p1,p2}, Roof -> p3. A reader that ignores xlinked  -->
<!-- boundedBy leaves the solid faces semantically untagged. Coordinates are   -->
<!-- the tetrahedron of building_with_materials.gml. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:app="http://www.opengis.net/citygml/appearance/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BX">
			<bldg:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p0">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p1">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p2">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p3">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</bldg:lod2Solid>
			<bldg:boundedBy>
				<bldg:GroundSurface gml:id="gs">
					<bldg:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember xlink:href="#p0"/>
						</gml:MultiSurface>
					</bldg:lod2MultiSurface>
				</bldg:GroundSurface>
			</bldg:boundedBy>
			<bldg:boundedBy>
				<bldg:WallSurface gml:id="ws">
					<bldg:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember xlink:href="#p1"/>
							<gml:surfaceMember xlink:href="#p2"/>
						</gml:MultiSurface>
					</bldg:lod2MultiSurface>
				</bldg:WallSurface>
			</bldg:boundedBy>
			<bldg:boundedBy>
				<bldg:RoofSurface gml:id="rs">
					<bldg:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember xlink:href="#p3"/>
						</gml:MultiSurface>
					</bldg:lod2MultiSurface>
				</bldg:RoofSurface>
			</bldg:boundedBy>
		</bldg:Building>
	</cityObjectMember>
</CityModel>
