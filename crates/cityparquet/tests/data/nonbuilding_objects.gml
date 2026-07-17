<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (CG-7: non-building 1st-level types). -->
<!-- A WaterBody with a lod2Solid (tetrahedron) and a class attribute, plus a    -->
<!-- LandUse with a lod1MultiSurface (two triangles). A reader that only streams  -->
<!-- bldg:Building emits NO features for this file. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns:gen="http://www.opengis.net/citygml/generics/2.0"
           xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0"
           xmlns:luse="http://www.opengis.net/citygml/landuse/2.0"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<wtr:WaterBody gml:id="W1">
			<gen:stringAttribute name="usage">
				<gen:value>leisure</gen:value>
			</gen:stringAttribute>
			<wtr:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember>
								<gml:Polygon>
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
								<gml:Polygon>
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
								<gml:Polygon>
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
								<gml:Polygon>
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
			</wtr:lod2Solid>
		</wtr:WaterBody>
	</cityObjectMember>
	<cityObjectMember>
		<luse:LandUse gml:id="L1">
			<luse:lod1MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon>
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
						<gml:Polygon>
							<gml:exterior>
								<gml:LinearRing>
									<gml:pos>10.0 0.0 0.0</gml:pos>
									<gml:pos>10.0 10.0 0.0</gml:pos>
									<gml:pos>0.0 10.0 0.0</gml:pos>
									<gml:pos>10.0 0.0 0.0</gml:pos>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</luse:lod1MultiSurface>
		</luse:LandUse>
	</cityObjectMember>
</CityModel>
